/// here's a list of llvm intrinsic https://docs.rs/llvmint/0.0.3/src/llvmint/lib.rs.html#23120
extern "C" {
    #[allow(dead_code)]
    #[link_name = "llvm.x86.clwb"]
    fn clwb(ptr: *const u8);

    // https://docs.rs/llvmint/0.0.3/llvmint/x86/fn.avx512_movntdqa.html
    #[link_name = "llvm.x86.avx512.movntdqa"]
    #[allow(improper_ctypes)]
    fn nt_load512(src: *const i8) -> std::arch::x86_64::__m512i;
}

/// Copy data from src to dst with nontemporal hint
/// Src and dst must be aligned on/multiple of a 64-byte boundary or a general-protection exception may be generated.
pub unsafe fn nt_copy_nonoverlapping(src: *const u8, dst: *mut u8, size: usize) {
    if cfg!(target_feature = "avx512f") {
        use std::arch::x86_64::_mm512_stream_si512;
        for i in (0..size).step_by(64) {
            let tmp = nt_load512(src.add(i) as *const i8);
            _mm512_stream_si512(dst.add(i) as *mut i64, tmp);
        }
    } else {
        std::ptr::copy_nonoverlapping(src, dst, size);
    }
}
