use std::{alloc, mem};

use crate::{
    async_task::Prefetcher,
    cache_manager::{
        clock_cache::oid::{LockedOid, OidWriteGuard},
        Rid,
    },
    utils::{
        obj_alloc::{align_up, HeapArray},
        PointerRef,
    },
};

pub type OidPointer = PointerRef<LockedOid>;

impl Clone for OidPointer {
    fn clone(&self) -> Self {
        Self::new(self.to_ptr())
    }
}

impl Copy for OidPointer {}

pub(crate) struct OidArray {
    heap: HeapArray<LockedOid, 64>,
}

impl OidArray {
    const ARENA_SIZE: usize = HeapArray::<LockedOid, 64>::ARENA_SIZE;

    pub(crate) fn new(cap: usize) -> Self {
        let size = cap * mem::size_of::<LockedOid>();

        let size = size + Self::ARENA_SIZE * 128;
        let size = align_up(size, Self::ARENA_SIZE);
        let layout = alloc::Layout::from_size_align(size, mem::align_of::<LockedOid>()).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };

        println!("oid array alloc, ptr: {:?}, size: {}", ptr, size);
        let heap = unsafe { HeapArray::new(size / Self::ARENA_SIZE, ptr) };

        Self { heap }
    }

    /// To reset the oid to 0, this is for benchmark only
    pub(crate) fn reset(&mut self) {
        let ptr = self.heap.start_addr() as *mut u8;
        let size = self.heap.capacity() * std::mem::size_of::<LockedOid>();

        unsafe {
            ptr.write_bytes(0, size);
        }

        self.heap = unsafe { HeapArray::new(size / Self::ARENA_SIZE, ptr) };
        println!("oid array reset, ptr: {:?}, size: {}", ptr, size);
    }

    pub(crate) fn capacity(&self) -> usize {
        self.heap.capacity()
    }

    #[inline]
    pub async fn get(&self, rid: Rid) -> &LockedOid {
        let ptr = self.heap.index_to_ptr(rid.as_u32());

        Prefetcher::fetch_ptr(ptr as *const i8).await;

        unsafe { &*ptr }
    }

    pub fn get_sync(&self, rid: Rid) -> &LockedOid {
        let ptr = self.heap.index_to_ptr(rid.as_u32());
        unsafe { &*ptr }
    }

    /// TODO: I know this is very strange, because we don't really need to store the rid here
    pub fn alloc_rid(&self) -> (Rid, OidWriteGuard) {
        let oid = LockedOid::from_rid(Rid::from_u32(0));
        let rid = Rid::from_u32(self.heap.append(oid).expect("oid run out of space, err:"));
        let write_guard = self.get_sync(rid).write();
        write_guard.store_rid(&rid);
        (rid, write_guard)
    }
}

impl Drop for OidArray {
    fn drop(&mut self) {
        let ptr = self.heap.start_addr() as *mut u8;
        let size = self.heap.capacity() * std::mem::size_of::<LockedOid>();
        let layout = alloc::Layout::from_size_align(size, mem::align_of::<LockedOid>()).unwrap();
        unsafe {
            alloc::dealloc(ptr, layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossbeam_utils::thread::scope;

    #[test]
    fn oid_multi_insert_delete() {
        let thread_cnt = 4;
        let oid_per_thread = 10000;

        let oid_array = OidArray::new(thread_cnt * oid_per_thread);

        scope(|s| {
            for _ in 0..thread_cnt {
                s.spawn(|_| {
                    for _k in 0..oid_per_thread {
                        let _rid = oid_array.alloc_rid();
                    }
                });
            }
        })
        .unwrap();
    }
}
