mod cache_inner;
mod node_meta;
pub mod oid;

#[cfg(test)]
mod tests;

use crate::{cache_manager::Rid, error::TransactionError, query::FieldsMeta};

use super::{QueryValue, Schema};
use cache_inner::CacheInner;
use metric::CtxCounter;

use oid::{OidReadGuard, OidWriteGuard};

pub struct ClockCache<S: Schema> {
    pub(crate) inner: CacheInner<S>,
}

impl<S> ClockCache<S>
where
    S: Schema,
{
    pub fn new(
        cache_size: usize,
        probe_len: usize,
        probe_rng: f32,
        storage_size: usize,
        schema: S,
        metric_ctx: Option<CtxCounter>,
    ) -> ClockCache<S> {
        Self {
            inner: CacheInner::new(
                cache_size,
                probe_rng,
                probe_len,
                storage_size,
                6,
                schema,
                metric_ctx,
            ),
        }
    }

    pub fn schema(&self) -> &S {
        self.inner.schema()
    }

    pub fn reset(&mut self) {
        self.inner.reset()
    }

    pub fn peek(&self, index: usize) -> Option<&S::Field> {
        self.inner.peek(index)
    }

    pub fn thread_init(&self) {
        self.inner.thread_init();
    }

    pub fn insert(&self, val: S::Tuple) -> (Rid, OidWriteGuard<'_>) {
        let (rid, mut write_guard) = self.inner.oid_array.alloc_rid();
        self.inner.insert(&mut write_guard, val);

        (rid, write_guard)
    }

    /// This is not an async function because read using oid will not touch any cold memory,
    /// instead, it just return the reference, the caller can prefetch().await if they believe it's a cache miss
    #[inline]
    pub fn read<'a, const N: usize>(
        &'a self,
        oid_read: &'a OidReadGuard<'a>,
        query: &'a FieldsMeta<N>,
    ) -> QueryValue<'a, S::Field, S::Tuple, OidReadGuard<'a>> {
        self.inner.read(oid_read, query)
    }

    #[inline]
    pub async fn read_lock(&self, rid: Rid) -> Result<OidReadGuard<'_>, TransactionError> {
        let ptr = self.inner.oid_array.get(rid).await;
        ptr.try_read()
    }

    #[inline]
    pub fn read_lock_sync(&self, rid: Rid) -> Result<OidReadGuard<'_>, TransactionError> {
        let ptr = self.inner.oid_array.get_sync(rid);
        ptr.try_read()
    }

    #[inline]
    pub async fn write_lock(&self, rid: Rid) -> Result<OidWriteGuard<'_>, TransactionError> {
        let ptr = self.inner.oid_array.get(rid).await;
        ptr.try_write()
    }

    #[inline]
    pub fn write_lock_sync(&self, rid: Rid) -> Result<OidWriteGuard<'_>, TransactionError> {
        let ptr = self.inner.oid_array.get_sync(rid);
        ptr.try_write()
    }
}
