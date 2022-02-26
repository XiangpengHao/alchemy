use thiserror::Error;

/// Internal unrecoverable system error
#[derive(Debug, Error)]
pub(crate) enum SystemError {
    #[error("failed to allocate pm file")]
    FileAlloc,
    #[error("failed to mmap the pm file")]
    MmapFail,
    #[error("mmap mapped to a different address")]
    MmapMismatchAddr,
    #[error("array allocator out of memory")]
    ArrayAllocatorOutOfMemory,
}

#[derive(Debug, Error)]
pub enum TransactionError {
    #[error("failed to acquire read lock")]
    ReadLockBusy = 0,
    #[error("failed to acquire write lock")]
    WriteLockBusy = 1,
    #[error("failed to upgrade lock")]
    UpgradeLockBusy = 2,
    #[error("failed to find the key in index")]
    IndexNotFound = 3,
    #[error("failed to modify the lock, maybe someone is acquiring it")]
    LockBusy = 4,
    #[error("user aborted the transaction")]
    UserAbort = 5,
}
