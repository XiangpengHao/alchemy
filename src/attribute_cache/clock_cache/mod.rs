mod cache_inner;
mod node_meta;
pub mod oid;

#[cfg(test)]
mod tests;

use crate::{
    async_task::Prefetcher,
    attribute_cache::{clock_cache::cache_inner::ClockNode, Rid},
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
    inner: CacheInner<S::Field>,
    storage: Storage<S::Tuple>,
    oid_array: OidArray,
    schema: S,
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
        let inner = CacheInner::new(cache_size, probe_rng, probe_len, metric_ctx);
        let tuple_cnt = storage_size / std::mem::size_of::<S::Tuple>();
        let oid_array = OidArray::new(tuple_cnt);
        let storage = Storage::new(oid_array.capacity());
        Self {
            inner,
            storage,
            oid_array,
            schema,
        }
    }

    #[inline]
    pub(crate) fn schema(&self) -> &S {
        &self.schema
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
        let cached_item = self.schema.to_cached(&val);
        let evicted = self
            .inner
            .insert(&mut write_guard, cached_item, &self.oid_array);

        if let Some(evicted) = evicted {
            let tuple = self.storage.get_mut(&evicted.0.rid());
            self.schema
                .write_back(unsafe { &*evicted.0.val.get() }, tuple);
        }

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
                if self.schema.matches(query) {
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
        let rid = match self.inner.read(oid) {
            Ok(entry) => {
                if self.schema.matches(query) {
                    return QueryValue::new(Some(&entry.val), None);
                } else {
                    counter!(Counter::ReadSchemaMiss, self.inner.metric_ctx);
                    let rid = entry.rid();
                    let tuple = self.storage.get(rid);
                    return QueryValue::new(Some(&entry.val), Some(tuple));
                }
            }
            Err(rid) => rid,
        };

        let tuple_rid = oid.to_rid();
        debug_assert_eq!(tuple_rid, rid);
        let tuple = self.storage.get(tuple_rid);

        counter!(Counter::ReadMiss, self.inner.metric_ctx);
        if self.inner.probe_len() == 0 {
            return QueryValue::new(None, Some(tuple));
        }

        Prefetcher::fetch(tuple).await;
        let cached_item = self.schema.to_cached(unsafe { &*tuple.get() });
        let new_node = ClockNode::new(cached_item, tuple_rid);
        if let Some((new_loc, evicted)) = self.inner.probe_and_replace_rng(
            new_node,
            tuple_rid,
            oid,
            self.inner.probe_rng as f64,
            &self.oid_array,
        ) {
            let entry = self.inner.get_entry(new_loc);
            entry.set_ref(self.inner.probe_rng);

            if let Some(evicted) = evicted {
                let tuple = self.storage.get_mut(&evicted.0.rid());
                self.schema
                    .write_back(unsafe { &*evicted.0.val.get() }, tuple);
            }

            if self.schema.matches(query) {
                return QueryValue::new(Some(&entry.val), None);
            } else {
                counter!(Counter::ReadSchemaMiss, self.inner.metric_ctx);
                let rid = entry.rid();
                let tuple = self.storage.get(rid);
                return QueryValue::new(Some(&entry.val), Some(tuple));
            }
        } else {
            return QueryValue::new(None, Some(tuple));
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
