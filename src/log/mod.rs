use std::{
    ffi::CString, os::unix::prelude::OsStrExt, path::PathBuf, str::FromStr, sync::Arc, thread,
};

#[cfg(all(feature = "fuzz", test))]
use shuttle::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(all(feature = "fuzz", test)))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;

use crate::{
    storage::{pm_heap::PmHeap, Storage},
    utils::{backoff::Backoff, nt_copy::nt_copy_nonoverlapping},
};

const LOG_FILE: &str = "target/log/";
const DEFAULT_BUFFER_SIZE: usize = 1024 * 1024 * 64;

const MAX_LOG_LEN: usize = 512;
const MIN_BUFF_ALIGN: usize = 512;

/// The logger is benchmark only, it has some issues regarding correctness:
/// 1. The flushing thread need to ensure the logging thread finished writing before flush the log entries to disk
///    One possible solution is to add epoch based logging flush
/// 2. ...
pub struct Logger {
    lsn: CachePadded<AtomicUsize>,
    cur_offset: CachePadded<AtomicUsize>,
    flushed_offset: AtomicUsize,
    buffer_size: usize,
    should_stop: AtomicBool,
    raw_file: i32,
    pm_buffer: *mut u8,
    buffers: [*mut u8; 2],
}

unsafe impl Send for Logger {}
unsafe impl Sync for Logger {}

impl Default for Logger {
    fn default() -> Self {
        Self::with_buffer_size(DEFAULT_BUFFER_SIZE)
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        let layout = std::alloc::Layout::from_size_align(self.buffer_size, MIN_BUFF_ALIGN).unwrap();
        for b in self.buffers {
            unsafe {
                std::alloc::dealloc(b, layout);
            }
        }
    }
}

impl Logger {
    pub fn new() -> Self {
        Self::with_buffer_size(DEFAULT_BUFFER_SIZE)
    }

    pub fn with_buffer_size(buffer_size: usize) -> Self {
        let log_dir = std::env::var("LOG_DIR").unwrap_or_else(|_| LOG_FILE.to_string());
        let path = PathBuf::from_str(&log_dir).unwrap().join("test.log");

        if path.exists() {
            std::fs::remove_file(path.clone()).unwrap();
        }
        std::fs::create_dir_all(log_dir).unwrap();

        let buffers = unsafe {
            let layout = std::alloc::Layout::from_size_align(buffer_size, MAX_LOG_LEN).unwrap();
            [std::alloc::alloc(layout), std::alloc::alloc(layout)]
        };

        let raw_file_id = {
            let path_str_c = CString::new(path.as_os_str().as_bytes()).unwrap();
            let file_id = unsafe {
                libc::open(
                    path_str_c.as_ptr(),
                    libc::O_CREAT | libc::O_WRONLY | libc::O_APPEND | libc::O_DIRECT,
                    libc::S_IRWXU,
                )
            };
            if file_id <= 0 {
                unsafe {
                    libc::perror("Failed to create the logging file:".as_ptr() as *const i8);
                }
            }
            file_id
        };

        Self {
            lsn: CachePadded::new(AtomicUsize::new(0)),
            cur_offset: CachePadded::new(AtomicUsize::new(0)),
            buffers,
            buffer_size,
            raw_file: raw_file_id,
            pm_buffer: PmHeap::get().alloc_frame(60_580_196_864),
            flushed_offset: AtomicUsize::new(0),
            should_stop: AtomicBool::new(false),
        }
    }

    /// Reserve a slot in the log file
    pub fn acquire_lsn(&self) -> usize {
        self.lsn.fetch_add(1, Ordering::Acquire)
    }

    /// A dedicated thread for flushing the logs
    /// Other threads will write to the buffer,
    /// when buffer is full, this thread will write to the file
    pub fn try_flush_logs(&self) -> Result<usize, usize> {
        let cur_offset = self.cur_offset.load(Ordering::Acquire);
        let flushed = self.flushed_offset.load(Ordering::Acquire);

        let distance = cur_offset - flushed;
        if distance < self.buffer_size {
            // The buffer is not full
            return Err(cur_offset);
        }

        assert!(
            distance < self.buffer_size * self.buffers.len(),
            "Buffer is growing too fast, the logging device can't keep up!"
        );
        let buffer_id = (flushed / self.buffer_size) % self.buffers.len();
        self.flush_buffer(buffer_id);
        // self.flush_buffer_pm(buffer_id, flushed);
        let flushed = self
            .flushed_offset
            .fetch_add(self.buffer_size, Ordering::SeqCst);

        Ok(flushed)
    }

    fn flush_buffer_pm(&self, buffer_id: usize, offset: usize) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.buffers[buffer_id] as *const u8,
                self.pm_buffer.add(offset),
                self.buffer_size,
            );
        }
    }

    fn flush_buffer(&self, buffer_id: usize) {
        #[cfg(not(feature = "fuzz"))]
        unsafe {
            let rt = libc::write(
                self.raw_file,
                self.buffers[buffer_id] as *const libc::c_void,
                self.buffer_size,
            );
            if rt <= 0 {
                libc::perror("Error encountered while write".as_ptr() as *const i8);
            }

            let rt = libc::fsync(self.raw_file);
            assert_eq!(rt, 0);
        }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn acquire_log_buffer(&self, size: usize) -> &mut [u8] {
        debug_assert!(size <= MAX_LOG_LEN);

        let loc = self.cur_offset.fetch_add(size, Ordering::AcqRel);
        let offset = loc % self.buffer_size;
        let buffer_id = (loc / self.buffer_size) % self.buffers.len();
        unsafe {
            let dst = self.buffers[buffer_id].add(offset);
            std::slice::from_raw_parts_mut(dst, size)
        }
    }

    /// The transaction thread append a log to the buffer
    pub fn append_log(&self, val: &[u8]) {
        let dst = self.acquire_log_buffer(val.len());
        dst.copy_from_slice(val);
    }

    pub fn stop_logger(&self) {
        self.should_stop.store(true, Ordering::Release);
    }
}

pub fn start_logger_thread(logger: Arc<Logger>) {
    thread::spawn(move || {
        let backoff = Backoff::new();
        while !logger.should_stop.load(Ordering::Relaxed) {
            if logger.try_flush_logs().is_err() {
                backoff.spin();
            } else {
                backoff.reset();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    #[cfg(not(feature = "fuzz"))]
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        thread,
    };

    #[cfg(feature = "fuzz")]
    use shuttle::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        thread,
    };

    use crate::{log::Logger, utils::backoff::Backoff};

    #[test]
    fn basic() {
        let log_size = 64;
        let buffer_size = log_size * 32;

        let logger = Arc::new(Logger::with_buffer_size(buffer_size));
        let log_entry = 127;
        let finished = Arc::new(AtomicUsize::new(0));

        let logger_thread = 3;
        let mut handler = vec![];

        for _i in 0..logger_thread {
            let logger = logger.clone();
            let finished = finished.clone();
            handler.push(thread::spawn(move || {
                let mut buffer = Vec::with_capacity(log_size);
                for i in 0..log_size {
                    buffer.push(i as u8);
                }

                for _i in 0..log_entry {
                    let _lsn = logger.acquire_lsn();
                    logger.append_log(&buffer);
                    std::thread::sleep(Duration::from_millis(6));
                }

                finished.fetch_add(1, Ordering::SeqCst);
            }));
        }

        let mut flush_cnt = 0;
        let backoff = Backoff::new();
        while finished.load(Ordering::SeqCst) != logger_thread {
            if let Err(_) = logger.try_flush_logs() {
                backoff.snooze();
            } else {
                flush_cnt += 1;
                backoff.reset();
            }
        }

        for h in handler.into_iter() {
            h.join().unwrap();
        }

        let log_bytes = log_entry * logger_thread * log_size;

        let expected_flush = log_bytes / buffer_size;
        let expected_diff = flush_cnt as isize - expected_flush as isize;
        assert!(expected_diff >= 0);
        assert!(expected_diff <= 1);

        let expected_lsn = log_entry * logger_thread;
        assert_eq!(expected_lsn, logger.lsn.load(Ordering::SeqCst));
    }

    #[cfg(feature = "fuzz")]
    #[test]
    fn fuzz_basic() {
        shuttle::check_pct(basic, 100_000, 3);
    }
}
