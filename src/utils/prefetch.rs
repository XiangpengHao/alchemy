use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};

const CACHE_LINE_SIZE: usize = 64;

pub(crate) fn mem_prefetch(addr: *const i8) {
    unsafe {
        if cfg!(miri) {
            // miri doesn't support prefetching intrinsics
        } else {
            _mm_prefetch(addr, _MM_HINT_T0);
        }
    }
}

#[allow(dead_code)]
pub(crate) fn mem_prefetch_n(addr: *const i8, size: usize) {
    for i in (0..size).step_by(CACHE_LINE_SIZE) {
        mem_prefetch(unsafe { addr.add(i) });
    }
}
