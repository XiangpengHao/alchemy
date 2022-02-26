pub(crate) mod mmap;
pub(crate) mod obj_alloc;
pub mod prefetch;
pub mod test_gen;
use std::sync::atomic::{AtomicU64, Ordering};
pub use test_gen::TestGen;
pub(crate) mod backoff;
pub(crate) mod nt_copy;

thread_local! {
    static TID: AtomicU64 = AtomicU64::new(0);
}

pub fn get_tid() -> u64 {
    TID.with(|t| {
        let old = t.load(Ordering::Relaxed);
        if old == 0 {
            let new_tid = std::thread::current().id().as_u64().get();
            t.store(new_tid, Ordering::Relaxed);
            new_tid
        } else {
            old
        }
    })
}

/// A wrapper around the raw pointer, so it implements Send and Sync
#[derive(Clone, Copy)]
pub struct PointerRef<T> {
    ptr: *mut T,
}

impl<T> PointerRef<T> {
    pub fn new(ptr: *mut T) -> Self {
        Self { ptr }
    }

    pub fn to_ptr(&self) -> *mut T {
        self.ptr
    }
}

unsafe impl<T> Send for PointerRef<T> {}
unsafe impl<T> Sync for PointerRef<T> {}
