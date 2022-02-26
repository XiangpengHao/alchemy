use flurry::HashMap;
use once_cell::sync::OnceCell;

use crate::utils::mmap::{create_and_map_pool, unmap_memory};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

pub(crate) struct PmHeap {
    file_id: AtomicUsize,
    file_mapping: HashMap<usize, (PathBuf, usize)>,
}

impl PmHeap {
    fn new() -> Self {
        Self {
            file_id: AtomicUsize::new(0),
            file_mapping: HashMap::new(),
        }
    }

    pub(crate) fn get() -> &'static PmHeap {
        static PM_HEAP: OnceCell<PmHeap> = OnceCell::new();
        PM_HEAP.get_or_init(PmHeap::new)
    }

    pub(crate) fn alloc_frame(&self, size: usize) -> *mut u8 {
        static POOL_PATH: OnceCell<String> = OnceCell::new();
        let pool_path = POOL_PATH
            .get_or_init(|| {
                std::env::var("POOL_DIR").unwrap_or_else(|_| "target/memory_pool".to_string())
            })
            .as_str();

        std::fs::create_dir_all(pool_path).unwrap();
        let file_id = self.file_id.fetch_add(1, Ordering::Relaxed);
        let file_path = Path::new(pool_path).join(format!("caching-{}.pool", file_id));

        let addr =
            create_and_map_pool(file_path.as_os_str().to_str().unwrap(), size, None).unwrap();

        self.file_mapping
            .pin()
            .insert(addr as usize, (file_path, size));

        addr
    }

    pub(crate) unsafe fn dealloc_frame(&self, ptr: *mut u8, size: usize) {
        let pinned_map = self.file_mapping.pin();
        let frame_meta = pinned_map.remove(&(ptr as usize)).unwrap();
        unmap_memory(ptr, size);
        std::fs::remove_file(frame_meta.0.clone()).unwrap();
    }
}

impl Drop for PmHeap {
    fn drop(&mut self) {
        let pinned_map = self.file_mapping.pin();
        for (k, v) in pinned_map.into_iter() {
            let _rv = std::fs::remove_file(v.0.clone());
            unsafe {
                unmap_memory(*k as *mut u8, v.1);
            }
        }
    }
}
