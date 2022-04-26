#![feature(negative_impls)]
#![feature(vec_into_raw_parts)]
#![feature(thread_id_value)]
#![feature(link_llvm_intrinsics)]
#![feature(stdsimd)]
#![feature(simd_ffi)]
#![feature(core_intrinsics)]

pub(crate) mod async_task;
pub mod attribute_cache;
pub mod error;
pub mod index;
pub mod log;
pub mod query;
pub(crate) mod storage;
pub mod table;
pub mod utils;

pub use async_task::{block_on, Executor};

pub use macro_utils::Tuple;
