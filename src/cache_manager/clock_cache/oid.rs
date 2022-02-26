use crate::{cache_manager::Rid, error::TransactionError};
use crossbeam_utils::Backoff;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fmt, usize};

/*
most significant                                                                                               least significant
 ┊                                                                                                                             ┊
 ┊                                                                                                                             ┊
 ┊                                                                                                                             ┊
║     7 byte    ║     6 byte    ║     5 byte    ║     4 byte    ║     3 byte    ║     2 byte    ║     1 byte    ║     0 byte    ║
╟───────────────╫───────────────╫───────────────╫───────────────╟───────────────╫───────────────╫───────────────╫───────────────╢
║▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒ ▒╢
╟─╫─────────────╫───────────────╟───────────────╟───────────────╟───────────────────────────────────────────────────────────────╢
 ┊             ┊        ┊                ┊             ┊                             id (cached id or row id)
 ┊             ┊        ┊                ┊             ┊
 ┊       Read lock   is_cache_id      is_dirty      hotness
 Write lock
*/
pub struct LockedOid {
    data: AtomicUsize,
}

const LOCK_MASK: usize = 0xff00_0000_0000_0000; // high 8 bits
const LOCK_WRITER: usize = 0x8000_0000_0000_0000; // highest bit
const LOCK_READER: usize = 0x0100_0000_0000_0000; // lowest lock bit
const MAX_READER: usize = (LOCK_MASK - LOCK_WRITER) / LOCK_READER;
const IS_CACHED: usize = 0x00ff_0000_0000_0000;
const ID_MASK: usize = 0x0000_0000_ffff_ffff;

/// Note that for this project only,
/// we don't implement the poison mechanism because we don't have time.
/// But implementing it shouldn't change performance and won't change the conclusions
impl LockedOid {
    pub fn from_rid(idx: Rid) -> Self {
        Self {
            data: AtomicUsize::new(idx.as_u32() as usize),
        }
    }

    pub fn from_cache(index: usize) -> Self {
        assert!(index & IS_CACHED == 0);
        Self {
            data: AtomicUsize::new(index | IS_CACHED),
        }
    }

    /// Blocking read
    pub fn read(&self) -> OidReadGuard {
        let backoff = Backoff::new();
        let mut val = self.data.load(Ordering::Relaxed);
        loop {
            let write_locked = (val & LOCK_WRITER) != 0;
            if write_locked {
                backoff.spin();
                val = self.data.load(Ordering::Relaxed);
                continue;
            }

            match self.data.compare_exchange_weak(
                val,
                val.wrapping_add(LOCK_READER),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return OidReadGuard::new(self),
                Err(v) => {
                    val = v;
                    backoff.spin();
                }
            }
        }
    }

    /// None blocking read
    pub fn try_read(&self) -> Result<OidReadGuard, TransactionError> {
        let val = self.data.load(Ordering::Relaxed);
        let write_locked = (val & LOCK_WRITER) != 0;
        if write_locked {
            return Err(TransactionError::ReadLockBusy);
        }

        debug_assert!(self.read_lock_cnt() + 1 < MAX_READER);

        match self.data.compare_exchange_weak(
            val,
            val.wrapping_add(LOCK_READER),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => Ok(OidReadGuard::new(self)),
            Err(_) => Err(TransactionError::LockBusy),
        }
    }

    pub fn read_unlock(&self) {
        debug_assert!({
            let val = self.data.load(Ordering::Relaxed);
            let write_locked = (val & LOCK_WRITER) != 0;
            debug_assert!(!write_locked);
            let read_locked_cnt = (val & LOCK_MASK) / LOCK_READER;
            read_locked_cnt > 0
        });

        self.data.fetch_sub(LOCK_READER, Ordering::Release);
    }

    /// Blocking write
    pub fn write(&self) -> OidWriteGuard {
        let backoff = Backoff::new();
        let mut val = self.data.load(Ordering::Relaxed);
        loop {
            let has_lock = (val & LOCK_MASK) != 0;
            if has_lock {
                backoff.spin();
                val = self.data.load(Ordering::Relaxed);
                continue;
            }

            match self.data.compare_exchange_weak(
                val,
                val | LOCK_WRITER,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return OidWriteGuard::new(self),
                Err(v) => {
                    val = v;
                    backoff.spin();
                }
            }
        }
    }

    /// None blocking write
    #[inline]
    pub fn try_write(&self) -> Result<OidWriteGuard, TransactionError> {
        let val = self.data.load(Ordering::Relaxed);
        let has_lock = (val & LOCK_MASK) != 0;
        if has_lock {
            return Err(TransactionError::WriteLockBusy);
        }
        match self.data.compare_exchange_weak(
            val,
            val | LOCK_WRITER,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => Ok(OidWriteGuard::new(self)),
            Err(_) => Err(TransactionError::LockBusy),
        }
    }

    pub fn write_unlock(&self) {
        debug_assert!({
            let val = self.data.load(Ordering::Relaxed);
            (val & LOCK_MASK) == LOCK_WRITER
        });

        self.data.fetch_sub(LOCK_WRITER, Ordering::Relaxed);
    }

    pub fn write_locked(&self) -> bool {
        let val = self.data.load(Ordering::Relaxed);
        (val & LOCK_WRITER) != 0
    }

    pub fn read_lock_cnt(&self) -> usize {
        let val = self.data.load(Ordering::Relaxed);
        (val & LOCK_MASK) / LOCK_READER
    }

    pub fn is_cached(&self) -> bool {
        self.data.load(Ordering::Relaxed) & IS_CACHED != 0
    }

    pub fn is_rid(&self) -> bool {
        !self.is_cached()
    }

    fn to_cache_index(&self) -> usize {
        debug_assert!(self.is_cached());
        let mut data = self.data.load(Ordering::Relaxed) & !LOCK_MASK;
        data &= !IS_CACHED;
        data
    }

    pub fn to_rid(&self) -> Rid {
        debug_assert!(self.is_rid());
        let data = self.data.load(Ordering::Relaxed) & !LOCK_MASK;
        Rid::from_u32((data & ID_MASK) as u32)
    }
}

impl fmt::Debug for LockedOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockedOid")
            .field("write_locked", &self.write_locked())
            .field("read_lock_cnt", &self.read_lock_cnt())
            .field("is_index", &self.is_cached())
            .finish()
    }
}

#[must_use = "if unused the lock will immediately unlock"]
pub struct OidReadGuard<'a> {
    lock: &'a LockedOid,
}

impl<'a> OidReadGuard<'a> {
    pub fn new(lock: &'a LockedOid) -> Self {
        OidReadGuard { lock }
    }

    pub fn to_cache_index(&self) -> usize {
        self.lock.to_cache_index()
    }

    pub fn raw_oid(&self) -> *mut LockedOid {
        self.lock as *const LockedOid as *mut LockedOid
    }

    pub fn get_index(&self) -> usize {
        debug_assert!(self.lock.is_cached());
        let mut data = self.lock.data.load(Ordering::Relaxed) & !LOCK_MASK;
        data &= !IS_CACHED;
        data
    }
}

impl Drop for OidReadGuard<'_> {
    fn drop(&mut self) {
        self.lock.read_unlock();
    }
}

#[must_use = "if unused the lock will immediately unlock"]
pub struct OidWriteGuard<'a> {
    lock: &'a LockedOid,
}

impl<'a> OidWriteGuard<'a> {
    pub fn new(lock: &'a LockedOid) -> Self {
        OidWriteGuard { lock }
    }

    pub fn raw_oid(&self) -> *mut LockedOid {
        self.lock as *const LockedOid as *mut LockedOid
    }

    pub fn to_cached(&self) -> usize {
        self.lock.to_cache_index()
    }

    pub fn store_rid(&self, rid: &Rid) {
        let mut data = self.lock.data.load(Ordering::Relaxed) & !ID_MASK;
        data &= !IS_CACHED; // unset the index mask
        data |= rid.as_u32() as usize;
        self.lock.data.store(data, Ordering::Relaxed);
    }

    pub fn store_index(&mut self, new: usize) {
        let mut data = self.lock.data.load(Ordering::Relaxed) & !ID_MASK;
        data |= IS_CACHED;
        data |= new;
        self.lock.data.store(data, Ordering::Relaxed);
    }
}

impl Drop for OidWriteGuard<'_> {
    fn drop(&mut self) {
        self.lock.write_unlock();
    }
}

impl<'a> !Send for OidReadGuard<'a> {}
impl<'a> !Send for OidWriteGuard<'a> {}

pub trait OidGuard<'a> {
    fn is_cached(&self) -> bool;
    fn to_cache_index(&self) -> usize;
    fn to_rid(&self) -> Rid;
    fn is_rid(&self) -> bool;

    /// Upgrading a lock is always non-blocking to prevent deadlock
    /// Returning error means that someone is also reading
    fn upgrade(self) -> Result<OidWriteGuard<'a>, OidReadGuard<'a>>;
}

impl<'a> OidGuard<'a> for OidReadGuard<'a> {
    fn is_cached(&self) -> bool {
        self.lock.is_cached()
    }
    fn to_cache_index(&self) -> usize {
        self.lock.to_cache_index()
    }

    fn to_rid(&self) -> Rid {
        self.lock.to_rid()
    }

    fn is_rid(&self) -> bool {
        self.lock.is_rid()
    }

    fn upgrade(self) -> Result<OidWriteGuard<'a>, OidReadGuard<'a>> {
        let val = self.lock.data.load(Ordering::Relaxed);
        let write_locked = (val & LOCK_WRITER) != 0;
        debug_assert!(!write_locked); // not write locked for sure because we're holding read lock.

        let read_locked_cnt = (val & LOCK_MASK) / LOCK_READER;
        if read_locked_cnt > 1 {
            // someone else is also holding read lock, just abort.
            return Err(self);
        }

        debug_assert!(read_locked_cnt == 1);

        let write_lock = (val & !LOCK_MASK) | LOCK_WRITER;
        match self.lock.data.compare_exchange_weak(
            val,
            write_lock,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                let lock = self.lock;
                std::mem::forget(self);
                Ok(OidWriteGuard::new(lock))
            }
            Err(_v) => Err(self),
        }
    }
}

impl<'a> OidGuard<'a> for OidWriteGuard<'a> {
    fn is_cached(&self) -> bool {
        self.lock.is_cached()
    }
    fn to_cache_index(&self) -> usize {
        self.lock.to_cache_index()
    }

    fn to_rid(&self) -> Rid {
        self.lock.to_rid()
    }

    fn is_rid(&self) -> bool {
        self.lock.is_rid()
    }

    fn upgrade(self) -> Result<OidWriteGuard<'a>, OidReadGuard<'a>> {
        Ok(self)
    }
}
