#![allow(clippy::question_mark)]
use crate::{
    async_task::Prefetcher,
    cache_manager::{Rid, Schema},
    storage::{oid_array::OidArray, Storage},
};
use metric::{counter, histogram, Counter, CtxCounter, Histogram};

use rand::Rng;
use std::{
    alloc,
    cell::UnsafeCell,
    mem,
    sync::atomic::{AtomicU32, Ordering},
};
use thread_local::ThreadLocal;

use super::{
    node_meta::NodeMeta,
    oid::{OidGuard, OidWriteGuard},
};

#[repr(C)]
pub(crate) struct ClockNode<T> {
    /// Technically speaking, rid should have type `Rid` instead of `AtomicU32`
    rid: AtomicU32,

    meta: NodeMeta,

    pub(super) val: UnsafeCell<T>,
}

impl<T> ClockNode<T> {
    fn new(val: T, rid: Rid) -> Self {
        Self {
            rid: AtomicU32::new(rid.as_u32()),
            meta: NodeMeta::new(),
            val: UnsafeCell::new(val),
        }
    }

    pub(super) fn rid(&self) -> Rid {
        let rid = self.rid.load(Ordering::Relaxed);
        debug_assert!(rid != u32::MAX);
        Rid::from_u32(rid)
    }

    fn set_ref(&self, prob: f32) {
        if rand::thread_rng().gen_bool(prob as f64) {
            self.meta.set_ref();
        }
    }

    // Clear the ref bit, return true if the ref bit is set
    fn try_clear_ref(&self) -> bool {
        self.meta.clear_if_set()
    }
}

#[repr(C)]
pub struct CacheInner<S: Schema>
where
    S::Tuple: 'static,
{
    capacity: u32,
    probe_len: u32,
    probe_rng: f32,
    pub(super) metric_ctx: Option<CtxCounter>,
    schema: S,
    tls_index: ThreadLocal<AtomicU32>,
    entries: *mut ClockNode<S::Field>,
}

impl<S: Schema> Drop for CacheInner<S> {
    fn drop(&mut self) {
        for i in 0..self.capacity as usize {
            unsafe {
                std::ptr::drop_in_place(self.entries.add(i));
            }
        }
        let layout = alloc::Layout::from_size_align(
            self.capacity as usize * mem::size_of::<ClockNode<S::Field>>(),
            mem::align_of::<ClockNode<S::Field>>(),
        )
        .unwrap();
        unsafe {
            alloc::dealloc(self.entries as *mut u8, layout);
        }
    }
}

impl<S: Schema> CacheInner<S> {
    pub(crate) fn new_cap(
        cache_cnt: usize,
        probe_len: usize,
        probe_rng: f32,
        schema: S,
        metric_ctx: Option<CtxCounter>,
    ) -> Self {
        assert!(cache_cnt > 0);

        let entries_size = cache_cnt * mem::size_of::<ClockNode<S::Field>>();
        let layout =
            alloc::Layout::from_size_align(entries_size, mem::align_of::<ClockNode<S::Field>>())
                .unwrap();
        let entries = unsafe { alloc::alloc_zeroed(layout) as *mut ClockNode<S::Field> };

        for i in 0..cache_cnt {
            let cur_entry = unsafe { &mut *entries.add(i) };
            cur_entry.rid = AtomicU32::new(u32::MAX);
        }

        CacheInner {
            entries,
            tls_index: ThreadLocal::new(),
            capacity: cache_cnt as u32,
            probe_len: probe_len as u32,
            probe_rng,
            schema,
            metric_ctx,
        }
    }

    /// To reset the oid to 0, this is for benchmark only
    pub fn reset(&mut self) {
        for i in 0..self.capacity as usize {
            let cur_entry = unsafe { &mut *self.entries.add(i) };
            cur_entry.rid = AtomicU32::new(u32::MAX);
        }
    }

    pub fn new(
        cache_size: usize,
        probe_rng: f32,
        probe_len: usize,
        schema: S,
        metric_ctx: Option<CtxCounter>,
    ) -> Self {
        let cap = cache_size / mem::size_of::<ClockNode<S::Field>>();
        CacheInner::new_cap(
            cap,
            probe_len,
            probe_rng,
            schema,
            metric_ctx,
        )
    }

    /// Init the tls_index so it will scatter around the whole entry space,
    /// instead of contending on a single slot
    pub(crate) fn thread_init(&self) {
        self.tls_index.get_or(|| {
            let mut rng = rand::thread_rng();

            let rng_idx = rng.gen_range(0..self.capacity as u32);
            AtomicU32::new(rng_idx)
        });
    }

    pub(crate) fn insert<'a>(
        &'a self,
        lock: &mut OidWriteGuard<'a>,
        val: &S::Tuple,
        storage: &Storage<S::Tuple>,
        oid_array: &OidArray,
    ) {
        assert!(lock.is_rid());
        let rid = lock.to_rid();

        let cached_item = self.schema.to_cached(&val);

        let clock_node = ClockNode::new(cached_item, rid);

        let _index = self.probe_and_replace_rng(clock_node, rid, lock, 1.0, storage, oid_array);
    }

    /// returns new index
    fn advance_index(&self) -> usize {
        let index = (self.get_index() + 1) % self.capacity();
        let idx = self.tls_index.get().unwrap();
        (*idx).store(index as u32, Ordering::Relaxed);
        index
    }

    /// Replace a old clock node with the new ones based on random probability
    /// return the new index if find a node within probe length
    fn probe_and_replace_rng<'a>(
        &'a self,
        new_node: ClockNode<S::Field>,
        rid: Rid,
        oid: &mut OidWriteGuard<'a>,
        probability: f64,
        storage: &Storage<S::Tuple>,
        oid_array: &OidArray,
    ) -> Option<usize> {
        if !rand::thread_rng().gen_bool(probability) {
            return None;
        }

        for p in 0..self.probe_len {
            let cur_index = self.advance_index();
            let cur_entry = self.get_entry_mut(cur_index);
            let cur_origin = cur_entry.rid.load(Ordering::Relaxed);

            if cur_origin == u32::MAX {
                // Validate phase
                match cur_entry.rid.compare_exchange_weak(
                    cur_origin,
                    rid.as_u32(),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        *cur_entry = new_node;
                        oid.store_index(cur_index);
                        histogram!(Histogram::ProbeLen, p as u64);
                        return Some(cur_index);
                    }
                    Err(_v) => continue,
                }
            } else {
                if cur_entry.try_clear_ref() {
                    continue;
                }

                let cur_rid = Rid::from_u32(cur_origin);
                let cur_oid = oid_array.get_sync(cur_rid);
                let oid_to_evict = cur_oid.try_write();
                if oid_to_evict.is_err() {
                    continue;
                }

                // Validate phase, make sure the entry.origin is not changed after we have the lock.
                if cur_entry
                    .rid
                    .compare_exchange_weak(
                        cur_origin,
                        cur_origin,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_err()
                {
                    continue;
                }

                let oid_to_evict = oid_to_evict.unwrap();
                oid_to_evict.store_rid(&cur_entry.rid());

                self.replace(cur_index, new_node, storage);

                oid.store_index(cur_index);

                histogram!(Histogram::ProbeLen, p as u64);
                return Some(cur_index);
            }
        }
        counter!(Counter::ProbeMiss, self.metric_ctx);
        None
    }

    pub(crate) async fn promote_entry<'a>(
        &'a self,
        oid: &mut OidWriteGuard<'a>,
        storage: &Storage<S::Tuple>,
        oid_array: &OidArray,
    ) -> Option<usize> {
        if oid.is_cached() {
            counter!(Counter::ReadHit, self.metric_ctx);
            return Some(oid.to_cache_index());
        }

        // We now hit a cache miss and prefetch the data on NVM
        let tuple_rid = oid.to_rid();
        let tuple = storage.get(tuple_rid);

        counter!(Counter::ReadMiss, self.metric_ctx);

        debug_assert!(oid.is_rid());

        if self.probe_len == 0 {
            return None;
        }

        Prefetcher::fetch(tuple).await;

        let cached_item = self.schema.to_cached(unsafe { &*tuple.get() });
        let new_node = ClockNode::new(cached_item, tuple_rid);

        self.probe_and_replace_rng(
            new_node,
            tuple_rid,
            oid,
            self.probe_rng as f64,
            storage,
            oid_array,
        )
    }

    /// Read with exclusive lock
    pub(super) async fn read_and_promote<'a: 'b, 'b>(
        &'a self,
        oid: &'b mut OidWriteGuard<'a>,
        storage: &Storage<S::Tuple>,
        oid_array: &OidArray,
    ) -> Result<&'a ClockNode<S::Field>, Rid> {
        counter!(Counter::ReadCnt, self.metric_ctx);

        let cache_idx = self.promote_entry(oid, storage, oid_array).await;

        match cache_idx {
            Some(idx) => {
                let entry = self.get_entry(idx);

                entry.set_ref(self.probe_rng);
                Ok(entry)
            }
            None => Err(oid.to_rid()),
        }
    }

    #[inline]
    pub(crate) fn read<'a, L: OidGuard<'a>>(
        &'a self,
        oid: &'a L,
    ) -> Result<&'a ClockNode<S::Field>, Rid> {
        counter!(Counter::ReadCnt, self.metric_ctx);

        if oid.is_cached() {
            let i = oid.to_cache_index();
            let entry = self.get_entry(i);

            entry.set_ref(self.probe_rng);

            counter!(Counter::ReadHit, self.metric_ctx);

            Ok(&entry)
        } else {
            counter!(Counter::ReadMiss, self.metric_ctx);
            debug_assert!(oid.is_rid());
            let tuple_rid = oid.to_rid();
            Err(tuple_rid)
        }
    }

    fn replace(&self, index: usize, new: ClockNode<S::Field>, storage: &Storage<S::Tuple>) {
        // when _old goes out of scope, the drop will persist the node if it's dirty
        let old = mem::replace(self.get_entry_mut(index), new);
        let tuple = storage.get_mut(&old.rid());

        self.schema.write_back(unsafe { &*old.val.get() }, tuple);
    }

    pub fn capacity(&self) -> usize {
        self.capacity as usize
    }

    #[allow(dead_code)]
    pub fn probe_len(&self) -> usize {
        self.probe_len as usize
    }

    fn get_entry(&self, index: usize) -> &ClockNode<S::Field> {
        unsafe { &*self.entries.add(index) }
    }

    #[allow(clippy::mut_from_ref)]
    fn get_entry_mut(&self, index: usize) -> &mut ClockNode<S::Field> {
        unsafe { &mut *self.entries.add(index) }
    }

    fn get_index(&self) -> usize {
        self.tls_index
            .get_or(|| AtomicU32::new(0))
            .load(Ordering::Relaxed) as usize
    }

    pub fn schema(&self) -> &S {
        &self.schema
    }
}
