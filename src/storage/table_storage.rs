use crate::{attribute_cache::Rid, utils::obj_alloc::poison_memory_region};
use std::{cell::UnsafeCell, mem};

use super::pm_heap::PmHeap;

pub(crate) struct Storage<T: 'static> {
    heap: *mut T,
    cap: usize,
}

impl<T: 'static> Storage<T> {
    /// Create a new table storage which has at most `cap` elements in it.
    /// The cap must be equal to (or larger than) the oid array cap
    pub(crate) fn new(cap: usize) -> Self {
        let size = cap * mem::size_of::<T>();
        let addr = PmHeap::get().alloc_frame(size);

        Self {
            heap: addr as *mut T,
            cap,
        }
    }

    pub(crate) fn get(&self, rid: Rid) -> &UnsafeCell<T> {
        let idx = rid.as_u32() as usize;
        debug_assert!(idx < self.cap);

        let ptr = unsafe { self.heap.add(idx) };
        let ptr = ptr as *const UnsafeCell<T>;
        unsafe { &*ptr }
    }

    #[allow(clippy::mut_from_ref)]
    pub(crate) fn get_mut(&self, rid: &Rid) -> &mut T {
        let idx = rid.as_u32() as usize;
        debug_assert!(idx < self.cap);

        let ptr = unsafe { self.heap.add(idx) };
        unsafe { &mut *ptr }
    }

    pub(crate) fn insert(&self, rid: &Rid, val: T) {
        unsafe { (self.get_mut(rid) as *mut T).write(val) }
    }
}

impl<T> Drop for Storage<T> {
    fn drop(&mut self) {
        let size = self.cap * mem::size_of::<T>();

        poison_memory_region(self.heap as *const u8, size);

        unsafe {
            PmHeap::get().dealloc_frame(self.heap as *mut u8, size);
        }
    }
}
