use crate::{attribute_cache::Rid, storage::oid_array::OidArray};
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
    pub(super) fn new(val: T, rid: Rid) -> Self {
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

    pub(super) fn set_ref(&self, prob: f32) {
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
pub(super) struct CacheInner<F> {
    capacity: u32,
    probe_len: u32,
    pub(super) probe_rng: f32,
    pub(super) metric_ctx: Option<CtxCounter>,
    tls_index: ThreadLocal<AtomicU32>,
    entries: *mut ClockNode<F>,
}

impl<F> Drop for CacheInner<F> {
    fn drop(&mut self) {
        for i in 0..self.capacity as usize {
            unsafe {
                std::ptr::drop_in_place(self.entries.add(i));
            }
        }
        let layout = alloc::Layout::from_size_align(
            self.capacity as usize * mem::size_of::<ClockNode<F>>(),
            mem::align_of::<ClockNode<F>>(),
        )
        .unwrap();
        unsafe {
            alloc::dealloc(self.entries as *mut u8, layout);
        }
    }
}

impl<F> CacheInner<F> {
    pub(crate) fn new_cap(
        cache_cnt: usize,
        probe_len: usize,
        probe_rng: f32,
        metric_ctx: Option<CtxCounter>,
    ) -> Self {
        assert!(cache_cnt > 0);

        let entries_size = cache_cnt * mem::size_of::<ClockNode<F>>();
        let layout =
            alloc::Layout::from_size_align(entries_size, mem::align_of::<ClockNode<F>>()).unwrap();
        let entries = unsafe { alloc::alloc_zeroed(layout) as *mut ClockNode<F> };

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
            metric_ctx,
        }
    }

    /// To reset the oid to 0, this is for benchmark only
    pub(super) fn reset(&mut self) {
        for i in 0..self.capacity as usize {
            let cur_entry = unsafe { &mut *self.entries.add(i) };
            cur_entry.rid = AtomicU32::new(u32::MAX);
        }
    }

    pub(super) fn new(
        cache_size: usize,
        probe_rng: f32,
        probe_len: usize,
        metric_ctx: Option<CtxCounter>,
    ) -> Self {
        let cap = cache_size / mem::size_of::<ClockNode<F>>();
        CacheInner::new_cap(cap, probe_len, probe_rng, metric_ctx)
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
        val: F,
        oid_array: &'a OidArray,
    ) -> Option<(ClockNode<F>, OidWriteGuard<'a>)> {
        assert!(lock.is_rid());
        let rid = lock.to_rid();

        let clock_node = ClockNode::new(val, rid);

        let index = self.probe_and_replace_rng(clock_node, rid, lock, 1.0, oid_array)?;
        index.1
    }

    /// returns new index
    #[inline]
    fn advance_index(&self) -> usize {
        let index = (self.get_index() + 1) % self.capacity();
        let idx = self.tls_index.get().unwrap();
        (*idx).store(index as u32, Ordering::Relaxed);
        index
    }

    /// Replace a old clock node with the new ones based on probability
    /// if find a node within probe length, return the new index as well as the old node
    #[allow(clippy::type_complexity)]
    pub(super) fn probe_and_replace_rng<'a>(
        &'a self,
        new_node: ClockNode<F>,
        rid: Rid,
        oid: &mut OidWriteGuard<'a>,
        probability: f64,
        oid_array: &'a OidArray,
    ) -> Option<(usize, Option<(ClockNode<F>, OidWriteGuard<'a>)>)> {
        if !rand::thread_rng().gen_bool(probability) {
            return None;
        }

        for p in 0..self.probe_len {
            let cur_index = self.advance_index();
            let cur_entry = self.get_entry_mut(cur_index);
            let cur_rid = cur_entry.rid.load(Ordering::Relaxed);

            if cur_rid == u32::MAX {
                // Validate phase
                match cur_entry.rid.compare_exchange_weak(
                    cur_rid,
                    rid.as_u32(),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        *cur_entry = new_node;
                        oid.store_index(cur_index);
                        histogram!(Histogram::ProbeLen, p as u64);
                        return Some((cur_index, None));
                    }
                    Err(_v) => continue,
                }
            } else {
                if cur_entry.try_clear_ref() {
                    continue;
                }

                let cur_rid = Rid::from_u32(cur_rid);
                let entry_oid = oid_array.get_sync(cur_rid);
                let oid_to_evict = entry_oid.try_write();
                if oid_to_evict.is_err() {
                    continue;
                }

                // Validate phase, make sure the entry.origin is not changed after we have the lock.
                if cur_entry
                    .rid
                    .compare_exchange_weak(
                        cur_rid.as_u32(),
                        cur_rid.as_u32(),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_err()
                {
                    continue;
                }

                let oid_to_evict = oid_to_evict.unwrap();
                oid_to_evict.store_rid(&cur_entry.rid());

                let old = mem::replace(self.get_entry_mut(cur_index), new_node);

                histogram!(Histogram::ProbeLen, p as u64);
                oid.store_index(cur_index);

                return Some((cur_index, Some((old, oid_to_evict))));
            }
        }
        counter!(Counter::ProbeMiss, self.metric_ctx);
        None
    }

    #[inline]
    pub(crate) fn read<'a, 'b, L: OidGuard<'a>>(
        &'a self,
        oid: &'b L,
    ) -> Result<&'a ClockNode<F>, Rid> {
        counter!(Counter::ReadCnt, self.metric_ctx);

        if oid.is_cached() {
            let i = oid.to_cache_index();
            let entry = self.get_entry(i);

            entry.set_ref(self.probe_rng);

            counter!(Counter::ReadHit, self.metric_ctx);

            Ok(entry)
        } else {
            counter!(Counter::ReadMiss, self.metric_ctx);
            debug_assert!(oid.is_rid());
            let tuple_rid = oid.to_rid();
            Err(tuple_rid)
        }
    }

    pub(super) fn capacity(&self) -> usize {
        self.capacity as usize
    }

    #[allow(dead_code)]
    pub(super) fn probe_len(&self) -> usize {
        self.probe_len as usize
    }

    pub(super) fn get_entry(&self, index: usize) -> &ClockNode<F> {
        unsafe { &*self.entries.add(index) }
    }

    #[allow(clippy::mut_from_ref)]
    fn get_entry_mut(&self, index: usize) -> &mut ClockNode<F> {
        unsafe { &mut *self.entries.add(index) }
    }

    fn get_index(&self) -> usize {
        self.tls_index
            .get_or(|| AtomicU32::new(0))
            .load(Ordering::Relaxed) as usize
    }
}
