mod cache_inner;
mod node_meta;
pub mod oid;

#[cfg(test)]
mod tests;

use crate::{
    cache_manager::Rid,
    error::TransactionError,
    query::FieldsMeta,
    storage::{oid_array::OidArray, Storage},
};

use self::oid::OidGuard;

use super::{QueryValue, Schema};
use cache_inner::CacheInner;
use metric::{counter, Counter, CtxCounter};

use oid::{OidReadGuard, OidWriteGuard};

pub(crate) struct ClockCache<S: Schema> {
    pub(crate) inner: CacheInner<S>,
    storage: Storage<S::Tuple>,
    oid_array: OidArray,
}

unsafe impl<S: Schema + Send> Send for ClockCache<S> {}
unsafe impl<S: Schema + Sync> Sync for ClockCache<S> {}

impl<S> ClockCache<S>
where
    S: Schema,
{
    pub(crate) fn new(
        cache_size: usize,
        probe_len: usize,
        probe_rng: f32,
        storage_size: usize,
        schema: S,
        metric_ctx: Option<CtxCounter>,
    ) -> ClockCache<S> {
        let inner = CacheInner::new(
            cache_size,
            probe_rng,
            probe_len,
            schema,
            metric_ctx,
        );
        let tuple_cnt = storage_size / std::mem::size_of::<S::Tuple>();
        let oid_array = OidArray::new(tuple_cnt);
        let storage = Storage::new(oid_array.capacity());
        Self {
            inner,
            storage,
            oid_array,
        }
    }

    #[inline]
    pub(crate) fn schema(&self) -> &S {
        self.inner.schema()
    }

    #[inline]
    pub(crate) fn reset(&mut self) {
        println!("resetting the table...");
        self.oid_array.reset();
        self.inner.reset()
    }

    #[inline]
    pub(crate) fn thread_init(&self) {
        self.inner.thread_init();
    }

    #[inline]
    pub(crate) fn insert(&self, val: S::Tuple) -> (Rid, OidWriteGuard<'_>) {
        counter!(Counter::InsertCnt, self.inner.metric_ctx);
        let (rid, mut write_guard) = self.oid_array.alloc_rid();
        self.inner
            .insert(&mut write_guard, &val, &self.storage, &self.oid_array);
        self.storage.insert(&rid, val);

        (rid, write_guard)
    }

    /// This is not an async function because read using oid will not touch any cold memory,
    /// instead, it just return the reference, the caller can prefetch().await if they believe it's a cache miss
    #[inline]
    pub(crate) fn read<'a, L: OidGuard<'a>, const N: usize>(
        &'a self,
        oid_read: &'a L,
        query: &'a FieldsMeta<N>,
    ) -> QueryValue<'a, S::Field, S::Tuple, L> {
        match self.inner.read(oid_read) {
            Ok(entry) => {
                if self.inner.schema().matches(query) {
                    QueryValue::new(Some(&entry.val), None)
                } else {
                    counter!(Counter::ReadSchemaMiss, self.inner.metric_ctx);
                    let rid = entry.rid();
                    let tuple = self.storage.get(rid);
                    QueryValue::new(Some(&entry.val), Some(tuple))
                }
            }
            Err(rid) => {
                let tuple = self.storage.get(rid);
                QueryValue::new(None, Some(tuple))
            }
        }
    }

    pub(crate) async fn read_and_promote<'a: 'b, 'b, const N: usize>(
        &'a self,
        oid: &'b mut OidWriteGuard<'a>,
        query: &FieldsMeta<N>,
    ) -> QueryValue<'b, S::Field, S::Tuple, OidWriteGuard<'a>> {
        match self
            .inner
            .read_and_promote(oid, &self.storage, &self.oid_array)
            .await
        {
            Ok(entry) => {
                if self.inner.schema().matches(query) {
                    QueryValue::new(Some(&entry.val), None)
                } else {
                    let rid = entry.rid();
                    counter!(Counter::ReadSchemaMiss, self.inner.metric_ctx);
                    let tuple = self.storage.get(rid);
                    QueryValue::new(Some(&entry.val), Some(tuple))
                }
            }
            Err(rid) => {
                let tuple = self.storage.get(rid);
                QueryValue::new(None, Some(tuple))
            }
        }
    }

    #[inline]
    pub(crate) async fn read_lock(&self, rid: Rid) -> Result<OidReadGuard<'_>, TransactionError> {
        let ptr = self.oid_array.get(rid).await;
        ptr.try_read()
    }

    #[inline]
    pub(crate) fn read_lock_sync(&self, rid: Rid) -> Result<OidReadGuard<'_>, TransactionError> {
        let ptr = self.oid_array.get_sync(rid);
        ptr.try_read()
    }

    #[inline]
    pub(crate) async fn write_lock(&self, rid: Rid) -> Result<OidWriteGuard<'_>, TransactionError> {
        let ptr = self.oid_array.get(rid).await;
        ptr.try_write()
    }

    #[inline]
    pub(crate) fn write_lock_sync(&self, rid: Rid) -> Result<OidWriteGuard<'_>, TransactionError> {
        let ptr = self.oid_array.get_sync(rid);
        ptr.try_write()
    }
}
