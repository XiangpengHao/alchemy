use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    mem::MaybeUninit,
    sync::atomic::{AtomicPtr, Ordering},
};

use crossbeam_utils::Backoff;
use flurry::HashMap;

/// Align the given address `size` upwards to alignment `align`.
///
/// Requires that `align` is a power of two.
pub(crate) fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) & !(align - 1)
}

/// A pre-faulted, zero-space-overhead, concurrent, wait-free allocator for an array of object T.
/// It's logically equivalent to allocating T multiple times from the global allocator
pub(crate) struct ArrayAllocator<T: Sized> {
    cap: usize,
    start: *const T,
    list_node: AtomicPtr<AtomicListNode>,
    phantom: PhantomData<T>,
}

#[derive(Debug)]
pub(crate) struct AtomicListNode {
    pub next: AtomicPtr<AtomicListNode>,
}

impl Default for AtomicListNode {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicListNode {
    pub(crate) fn new() -> Self {
        unsafe { MaybeUninit::zeroed().assume_init() }
    }

    /// casting a *u8 to *ListNode can be UB because of alignment
    #[allow(clippy::cast_ptr_alignment)]
    pub(crate) fn from_u8_ptr_unchecked(addr: *mut u8) -> *mut AtomicListNode {
        addr as *mut AtomicListNode
    }
}

unsafe impl<T> Send for ArrayAllocator<T> {}
unsafe impl<T> Sync for ArrayAllocator<T> {}

use std::mem;

use crate::error::SystemError;

use super::get_tid;

extern "C" {
    fn __asan_unpoison_memory_region(addr: *const u8, size: usize);
    fn __asan_poison_memory_region(addr: *const u8, size: usize);
}

pub(crate) fn poison_memory_region(addr: *const u8, size: usize) {
    if cfg!(feature = "sans") {
        unsafe {
            __asan_poison_memory_region(addr, size);
        }
    }
}

pub(crate) fn unpoison_memory_region(addr: *const u8, size: usize) {
    if cfg!(feature = "sans") {
        unsafe {
            __asan_unpoison_memory_region(addr, size);
        }
    }
}

impl<T> ArrayAllocator<T> {
    /// # Safety
    /// The data should be at least cap * mem::size_of::<T>().
    /// The only correct way to drop allocator is by calling `allocator.start_addr()`
    /// And reclaim the `data` to where it comes from
    pub(crate) unsafe fn new(cap: usize, data: *mut u8) -> Self {
        assert!(mem::size_of::<T>() >= mem::size_of::<AtomicListNode>());
        assert!(mem::align_of::<T>() >= mem::align_of::<AtomicListNode>());

        let mut node_n = AtomicListNode::default();

        for i in 0..cap {
            let block_start = data.add(i * mem::size_of::<T>());
            let list_node = AtomicListNode::from_u8_ptr_unchecked(block_start);
            list_node.write(node_n);

            node_n = AtomicListNode {
                next: AtomicPtr::new(list_node),
            };
        }

        let new_node = node_n.next.load(Ordering::Relaxed);

        poison_memory_region(data, cap);

        Self {
            cap,
            list_node: AtomicPtr::new(new_node),
            start: data as *const T,
            phantom: PhantomData,
        }
    }

    /// # Safety
    /// Only for benchmark purpose
    pub(crate) unsafe fn reset(&self) {
        let data = self.start as *mut u8;

        let mut node_n = AtomicListNode::default();

        for i in 0..self.cap {
            let block_start = data.add(i * mem::size_of::<T>());
            let list_node = AtomicListNode::from_u8_ptr_unchecked(block_start);
            list_node.write(node_n);

            node_n = AtomicListNode {
                next: AtomicPtr::new(list_node),
            };
        }

        let new_node = node_n.next.load(Ordering::Relaxed);

        poison_memory_region(data, self.cap);
        self.list_node.store(new_node, Ordering::Relaxed);
    }

    pub(crate) fn capacity(&self) -> usize {
        self.cap
    }

    pub(crate) fn start_addr(&self) -> *const T {
        self.start
    }

    #[inline(always)]
    pub(crate) fn alloc(&self) -> Result<(usize, &mut MaybeUninit<T>), SystemError> {
        let mut node = self.list_node.load(Ordering::Acquire);

        let backoff = Backoff::new();
        loop {
            if node.is_null() {
                return Err(SystemError::ArrayAllocatorOutOfMemory);
            }

            unpoison_memory_region(node as *const u8, mem::size_of::<T>());

            let next = unsafe { (*node).next.load(Ordering::Acquire) };
            match self
                .list_node
                .compare_exchange(node, next, Ordering::Acquire, Ordering::Acquire)
            {
                Ok(_) => {
                    let rv_ptr = node as usize;
                    let index = (rv_ptr - (self.start as usize)) / mem::size_of::<T>();
                    return Ok((index, unsafe { &mut *(node as *mut MaybeUninit<T>) }));
                }
                Err(v) => {
                    node = v;
                    backoff.spin();
                    continue;
                }
            }
        }
    }

    /// Safety
    /// # the index must be less or equal to capacity
    #[allow(dead_code)]
    pub(crate) unsafe fn get_by_index(&self, index: usize) -> &MaybeUninit<T> {
        debug_assert!(index < self.cap);

        let ptr = self.start.add(index);
        &mut *(ptr as *mut MaybeUninit<T>)
    }

    #[allow(dead_code)]
    pub fn dealloc(&self, ptr: *const T) {
        let mut next = self.list_node.load(Ordering::Acquire);

        let backoff = Backoff::new();

        loop {
            let new_node = AtomicListNode {
                next: AtomicPtr::new(next),
            };
            let new_node_ptr = AtomicListNode::from_u8_ptr_unchecked(ptr as *mut u8);
            unsafe {
                new_node_ptr.write(new_node);
            }

            match self.list_node.compare_exchange_weak(
                next,
                new_node_ptr,
                Ordering::Acquire,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    poison_memory_region(ptr as *const u8, mem::size_of::<T>());
                    break;
                }
                Err(v) => {
                    next = v;
                    backoff.spin();
                    continue;
                }
            }
        }
    }
}

/// Each thread will allocate from its own tls-allocator to reduce burden on one global slot
/// `N` specifies how many items the tls-allocator can hold, the larger N is,
/// the less contention on the global slot but at the cost of having more internal fragmentation
/// `HeapArray::ARENA_SIZE` is the size of tls allocator slab
pub(crate) struct HeapArray<T: 'static, const N: usize> {
    heap: ArrayAllocator<[T; N]>,
    tls_mapping: HashMap<u64, TlsAllocator<T>>,
}

impl<T: 'static, const N: usize> HeapArray<T, N> {
    pub const ARENA_SIZE: usize = N * mem::size_of::<T>();

    /// # Safety
    /// 1. The `ptr` should be at least cap * mem::size_of::<T>().
    /// 2. The caller need to properly reclaim the `ptr`
    pub(crate) unsafe fn new(arena_cnt: usize, ptr: *mut u8) -> Self {
        Self {
            heap: ArrayAllocator::new(arena_cnt, ptr),
            tls_mapping: HashMap::new(),
        }
    }

    fn get_tls_allocator<'a>(
        &'a self,
        guard: &'a flurry::epoch::Guard,
    ) -> Result<&'a TlsAllocator<T>, SystemError> {
        let thread_id = get_tid();

        let tls_map = self.tls_mapping.get(&thread_id, guard);
        match tls_map {
            Some(v) => Ok(v),
            None => {
                let arena = self.new_arena()?;
                let tls_allocator = TlsAllocator::new(arena);
                self.tls_mapping.insert(thread_id, tls_allocator, guard);
                Ok(self.tls_mapping.get(&thread_id, guard).unwrap())
            }
        }
    }

    fn new_arena(&self) -> Result<TlsArena<T>, SystemError> {
        let (_index, ptr) = self.heap.alloc()?;
        unsafe {
            Ok(TlsArena::new(
                Self::ARENA_SIZE / mem::size_of::<T>(),
                ptr.as_ptr() as *mut u8,
            ))
        }
    }

    pub(crate) fn append(&self, item: T) -> Result<u32, SystemError> {
        let guard = flurry::epoch::pin();
        let tls_allocator = match self.get_tls_allocator(&guard) {
            Ok(v) => v,
            Err(e) => return Err(e),
        };

        loop {
            match tls_allocator.alloc() {
                Ok(allocated) => {
                    allocated.write(item);
                    return Ok(self.ptr_to_index(allocated.as_mut_ptr()));
                }
                Err(_e) => {
                    let arena = match self.new_arena() {
                        Ok(v) => v,
                        Err(e) => return Err(e),
                    };
                    tls_allocator.update_arena(arena);
                    continue;
                }
            }
        }
    }

    pub(crate) fn ptr_to_index(&self, ptr: *mut T) -> u32 {
        let distance = (ptr as usize) - (self.heap.start_addr() as usize);
        let index = distance / mem::size_of::<T>();

        debug_assert!(index < self.capacity());
        debug_assert!(index < u32::MAX as usize);

        index as u32
    }

    #[inline]
    pub(crate) fn index_to_ptr(&self, index: u32) -> *mut T {
        debug_assert!((index as usize) < self.capacity());

        unsafe { (self.heap.start_addr() as *mut T).add(index as usize) }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.heap.capacity() * N
    }

    #[allow(dead_code)]
    pub(crate) fn delete(&self, idx: u32) {
        let guard = flurry::epoch::pin();
        let ptr = self.index_to_ptr(idx);
        let tls_allocator = self
            .get_tls_allocator(&guard)
            .expect("no tls allocator fund!");
        tls_allocator.dealloc(ptr);
    }

    pub(crate) fn start_addr(&self) -> *mut T {
        self.heap.start_addr() as *mut T
    }
}

#[cfg(test)]
mod test {
    use std::mem::MaybeUninit;

    use super::ArrayAllocator;

    fn allocator_gen(cap: usize) -> ArrayAllocator<usize> {
        let t = Vec::<usize>::with_capacity(cap);
        let (ptr, _len, _cap) = t.into_raw_parts();
        unsafe { ArrayAllocator::new(cap, ptr as *mut u8) }
    }

    #[test]
    fn alloc() {
        let cap = 1024;
        let allocator = allocator_gen(cap);

        let mut alloced = Vec::new();

        for i in 0..cap {
            let (index, val) = allocator.alloc().unwrap();
            assert_eq!(
                unsafe { allocator.get_by_index(index) } as *const MaybeUninit<usize>,
                val as *const MaybeUninit<usize>
            );

            let val = val.as_mut_ptr();
            unsafe {
                *val = i;
            }
            alloced.push(val);
        }

        for (i, v) in alloced.into_iter().enumerate() {
            assert_eq!(unsafe { *v }, i);
        }

        unsafe {
            Vec::from_raw_parts(allocator.start_addr() as *mut u8, 0, allocator.cap);
        }
    }

    #[test]
    fn dealloc() {
        let cap = 1024;
        let allocator = allocator_gen(cap);
        let mut allocated = Vec::new();

        for i in 0..cap {
            let val = allocator.alloc().unwrap().1.as_mut_ptr();
            unsafe {
                *val = i;
            }
            allocated.push(val);
        }

        for p in allocated.into_iter() {
            allocator.dealloc(p as *mut usize);
        }

        for _i in 0..cap {
            let val = allocator.alloc().unwrap().1.as_mut_ptr();
            unsafe {
                *val = 42;
            }
        }

        unsafe {
            Vec::from_raw_parts(allocator.start_addr() as *mut u8, 0, allocator.cap);
        }
    }

    #[test]
    #[should_panic(expected = "Allocator run out of space!")]
    fn test_alloc_oom() {
        let cap = 10;
        let t = Vec::<usize>::with_capacity(cap);
        let (ptr, _len, cap) = t.into_raw_parts();
        let allocator = unsafe { ArrayAllocator::<usize>::new(cap, ptr as *mut u8) };

        for _i in 0..(cap + 1) {
            let val = allocator
                .alloc()
                .expect("Allocator run out of space!")
                .1
                .as_mut_ptr();
            unsafe {
                *val = 42;
            }
        }
    }
}

struct ListNode {
    next: *mut ListNode,
}

struct TlsFreeList<T> {
    head: *mut ListNode,
    phantom: PhantomData<T>,
}
unsafe impl<T> Send for TlsFreeList<T> {}
unsafe impl<T> Sync for TlsFreeList<T> {}

impl<T> TlsFreeList<T> {
    fn pop(&mut self) -> Option<*mut u8> {
        if self.head.is_null() {
            None
        } else {
            let rt = self.head;
            self.head = unsafe { &*self.head }.next;
            Some(rt as *mut u8)
        }
    }

    fn insert(&mut self, ptr: *mut T) {
        let ptr = ptr as *mut ListNode;
        unsafe {
            ptr.write(ListNode { next: self.head });
        }
        self.head = ptr;
    }
}

pub(crate) struct TlsArena<T>(pub(crate) ArrayAllocator<T>);

impl<T> TlsArena<T> {
    pub(crate) unsafe fn new(cap: usize, ptr: *mut u8) -> Self {
        Self(ArrayAllocator::new(cap, ptr))
    }
}

/// Arena holds a block of continuos memory
/// Free list tracks the deallocated memory
/// When allocating memory, try to allocate from free list, then allocate from arena
struct TlsAllocatorInner<T> {
    arena: TlsArena<T>,
    free_list: TlsFreeList<T>,
}

unsafe impl<T> Send for TlsAllocatorInner<T> {}
unsafe impl<T> Sync for TlsAllocatorInner<T> {}

pub(crate) struct TlsAllocator<T> {
    inner: UnsafeCell<TlsAllocatorInner<T>>,
}

unsafe impl<T> Send for TlsAllocator<T> {}
unsafe impl<T> Sync for TlsAllocator<T> {}

impl<T> TlsAllocator<T> {
    pub(crate) fn alloc(&self) -> Result<&mut MaybeUninit<T>, SystemError> {
        let allocator = unsafe { &mut *self.inner.get() };

        if let Some(ptr) = allocator.free_list.pop() {
            return Ok(unsafe { &mut *(ptr as *mut MaybeUninit<T>) });
        }

        let ptr = allocator.arena.0.alloc()?;
        Ok(ptr.1)
    }

    pub(crate) fn update_arena(&self, arena: TlsArena<T>) {
        let allocator = unsafe { &mut *self.inner.get() };
        allocator.arena = arena;
    }

    pub(crate) fn new(arena: TlsArena<T>) -> Self {
        let allocator = TlsAllocatorInner {
            arena,
            free_list: TlsFreeList {
                head: std::ptr::null_mut(),
                phantom: PhantomData,
            },
        };
        Self {
            inner: UnsafeCell::new(allocator),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn dealloc(&self, ptr: *mut T) {
        let allocator = unsafe { &mut *self.inner.get() };
        allocator.free_list.insert(ptr);
    }
}
