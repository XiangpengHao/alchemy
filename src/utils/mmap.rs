use crate::error::SystemError;
use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::ptr;

pub(crate) fn create_and_map_pool(
    pool_path: &str,
    heap_size: usize,
    addr: Option<*const u8>,
) -> Result<*mut u8, SystemError> {
    let fd = create_and_allocate_file(pool_path, heap_size)?;

    let map_sync = map_nvm_builder(heap_size, addr, fd.as_raw_fd()).map();

    match map_sync {
        Ok(addr) => Ok(addr),
        Err(_) => {
            println!("Mapping with `MAP_SYNC` failed, are you using a NVM file? Try to map as a disk file instead...");
            let map_file = map_dram_builder(heap_size, addr, Some(fd.as_raw_fd()))
                .huge_page()
                .map();
            match map_file {
                Ok(addr) => Ok(addr),
                Err(_) => {
                    println!("Mapping with MAP_HUGETLB failed, falling back to use 4KB pages");
                    map_dram_builder(heap_size, addr, Some(fd.as_raw_fd())).map()
                }
            }
        }
    }
}

pub(crate) fn create_and_allocate_file(
    pool_path: &str,
    file_size: usize,
) -> Result<File, SystemError> {
    use std::fs::OpenOptions;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(true)
        .create(true)
        .open(pool_path)
        .map_err(|_e| SystemError::FileAlloc)?;

    file.set_len(file_size as u64)
        .map_err(|_e| SystemError::FileAlloc)?;

    file.sync_all().unwrap();
    Ok(file)
}

pub(crate) unsafe fn unmap_memory(addr: *mut u8, size: usize) {
    libc::munmap(addr as *mut libc::c_void, size);
}

pub(crate) struct MemoryMapper {
    heap_size: usize,
    addr: Option<*const u8>,
    fd: Option<i32>,
    flags: i32,
}

impl MemoryMapper {
    pub(crate) fn new(heap_size: usize) -> Self {
        MemoryMapper {
            heap_size,
            addr: None,
            fd: None,
            flags: libc::MAP_HUGE_2MB,
        }
    }

    pub(crate) fn addr(mut self, addr: *const u8) -> Self {
        self.flags |= libc::MAP_FIXED;
        self.addr = Some(addr);
        self
    }

    pub(crate) fn flags(mut self, flags: i32) -> Self {
        self.flags |= flags;
        self
    }

    pub(crate) fn huge_page(mut self) -> Self {
        self.flags |= libc::MAP_HUGETLB;
        self
    }

    /// Memory map an nvm-based file
    /// Use `MAP_SHARED_VALIDATE` and `MAP_SYNC` as specified here:
    /// https://man7.org/linux/man-pages/man2/mmap.2.html
    pub(crate) fn nvm_file(mut self, fd: i32) -> Self {
        self.flags |= libc::MAP_SYNC | libc::MAP_SHARED_VALIDATE;
        self.fd = Some(fd);
        self
    }

    pub(crate) fn disk_file(mut self, fd: i32) -> Self {
        self.fd = Some(fd);
        self
    }

    pub(crate) fn map(&self) -> Result<*mut u8, SystemError> {
        let mapped_addr: *mut u8 = unsafe {
            libc::mmap(
                self.addr.unwrap_or(ptr::null_mut()) as *mut libc::c_void,
                self.heap_size,
                libc::PROT_WRITE | libc::PROT_READ,
                self.flags,
                self.fd.unwrap_or(-1),
                0,
            ) as *mut u8
        };

        let heap_start = {
            if mapped_addr == libc::MAP_FAILED as *mut u8 {
                return Err(SystemError::MmapFail);
            } else if self.addr.is_some() && mapped_addr as usize != self.addr.unwrap() as usize {
                return Err(SystemError::MmapMismatchAddr);
            } else {
                mapped_addr
            }
        };
        Ok(heap_start)
    }
}

/// A helper to create builder for nvm mapping
pub(crate) fn map_nvm_builder(heap_size: usize, addr: Option<*const u8>, fd: i32) -> MemoryMapper {
    let mut builder = MemoryMapper::new(heap_size).nvm_file(fd);
    if let Some(addr) = addr {
        builder = builder.addr(addr);
    }
    builder
}

/// A helper to create builder for dram mapping
pub(crate) fn map_dram_builder(
    heap_size: usize,
    addr: Option<*const u8>,
    fd: Option<i32>,
) -> MemoryMapper {
    let mut builder = MemoryMapper::new(heap_size);
    if let Some(fd) = fd {
        builder = builder.disk_file(fd);
    }

    if let Some(addr) = addr {
        builder = builder.addr(addr);
    }

    builder.flags(libc::MAP_ANONYMOUS | libc::MAP_PRIVATE)
}
