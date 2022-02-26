use std::{
    alloc::Layout,
    marker::PhantomData,
    mem,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    cache_manager::{
        clock_cache::oid::{OidGuard, OidReadGuard, OidWriteGuard},
        ClockCache, QueryValue, Rid, Schema,
    },
    error::TransactionError,
    index::{Art, DbIndex},
    query::{FieldsMeta, UpdateQuery},
    storage::pm_heap::PmHeap,
    utils::{
        get_tid,
        nt_copy::nt_copy_nonoverlapping,
        obj_alloc::{align_up, poison_memory_region, ArrayAllocator},
    },
};
use flurry::HashMap;
use metric::CtxCounter;

pub struct DbTable<I, S>
where
    I: DbIndex,
    S: Schema,
{
    cache: ClockCache<S>,
    index: I,
}

pub struct DbTableBuilder<I, S>
where
    I: DbIndex,
    S: Schema,
{
    cache_size: Option<usize>,
    storage_size: Option<usize>,
    probe_len: Option<usize>,
    probe_rng: Option<f32>,
    index: Option<I>,
    schema: Option<S>,
    metric_context: Option<CtxCounter>,
    pt_schema: PhantomData<S>,
}

impl<I, S> Default for DbTableBuilder<I, S>
where
    I: DbIndex,
    S: Schema,
{
    fn default() -> Self {
        DbTableBuilder::new()
    }
}

impl<I, S> DbTableBuilder<I, S>
where
    I: DbIndex,
    S: Schema,
{
    pub fn new() -> Self {
        DbTableBuilder {
            cache_size: None,
            storage_size: None,
            probe_len: None,
            probe_rng: None,
            index: None,
            schema: None,
            metric_context: None,
            pt_schema: PhantomData,
        }
    }

    #[must_use]
    pub fn cache(mut self, size: usize) -> Self {
        self.cache_size = Some(size);
        self
    }

    #[must_use]
    pub fn storage(mut self, size: usize) -> Self {
        self.storage_size = Some(size);
        self
    }

    #[must_use]
    pub fn probe_len(mut self, probe_len: Option<usize>) -> Self {
        self.probe_len = probe_len;
        self
    }

    #[must_use]
    pub fn probe_rng(mut self, probe_rng: f32) -> Self {
        self.probe_rng = Some(probe_rng);
        self
    }

    #[must_use]
    pub fn index(mut self, index: I) -> Self {
        self.index = Some(index);
        self
    }

    #[must_use]
    pub fn schema(mut self, schema: S) -> Self {
        self.schema = Some(schema);
        self
    }

    #[must_use]
    pub fn metric_context(mut self, ctx: CtxCounter) -> Self {
        self.metric_context = Some(ctx);
        self
    }

    pub fn build(self) -> DbTable<I, S> {
        let cache_size = self.cache_size.expect("cache_size is not specified!");
        let probe = self.probe_len.unwrap_or(6);
        let probe_rng = self.probe_rng.unwrap_or(0.01);
        let storage = self.storage_size.expect("storage size is required!");

        let cap = storage / mem::size_of::<S::Tuple>();
        let index = self.index.unwrap_or_else(|| I::with_capacity(cap));
        let schema = self.schema.expect("cache schema is required");

        DbTable {
            cache: ClockCache::new(
                cache_size,
                probe,
                probe_rng,
                storage,
                schema,
                self.metric_context,
            ),
            index,
        }
    }
}

impl<S> DbTable<Art, S>
where
    S: Schema,
{
    pub fn range(&self, low: usize, high: usize, rid_buffer: &mut [Rid]) -> Option<usize> {
        let mut range_buffer = vec![0; rid_buffer.len()];
        let scanned = self.index.range(low, high, &mut range_buffer)?;
        for (i, v) in range_buffer.iter().take(scanned).enumerate() {
            rid_buffer[i] = Rid::from_u32(*v as u32);
        }
        Some(scanned)
    }
}

impl<I, S> DbTable<I, S>
where
    I: DbIndex,
    S: Schema,
{
    pub fn builder() -> DbTableBuilder<I, S> {
        DbTableBuilder::new()
    }

    pub fn reset(&mut self) {
        self.index = I::new();
        self.cache.reset();
    }

    #[must_use = "returns a x-locked guard"]
    pub fn insert<'a>(&'a self, record: S::Tuple, guard: &I::Guard) -> (Rid, OidWriteGuard<'a>) {
        let key = self.cache.schema().key(&record);
        let insert_rv = self.cache.insert(record);
        self.index.insert(key, insert_rv.0, guard);
        (insert_rv.0, insert_rv.1)
    }

    pub fn thread_init(&self) {
        self.cache.thread_init();
    }

    pub fn pin(&self) -> I::Guard {
        self.index.pin()
    }

    /// Read fields from provided oid
    /// it consumes the oid and either return the original read guard or upgraded write guard,
    /// the latter means the item is lifted to the cache during cache lookup.
    #[inline]
    pub fn read<'a, L: OidGuard<'a>, const N: usize>(
        &'a self,
        oid_guard: &'a L,
        query: &'a FieldsMeta<N>,
    ) -> QueryValue<'a, S::Field, S::Tuple, L> {
        self.cache.inner.read(oid_guard, query)
    }

    pub async fn read_and_promote<'a: 'b, 'b, const N: usize>(
        &'a self,
        oid_guard: &'b mut OidWriteGuard<'a>,
        query: &FieldsMeta<N>,
    ) -> QueryValue<'b, S::Field, S::Tuple, OidWriteGuard<'a>> {
        self.cache.inner.read_and_promote(oid_guard, query).await
    }

    pub async fn read_oid(
        &self,
        key: &usize,
        guard: &I::Guard,
    ) -> Result<OidReadGuard<'_>, TransactionError> {
        let idx = self
            .index
            .get(key, guard)
            .ok_or(TransactionError::IndexNotFound)?;
        self.cache.read_lock(idx).await
    }

    pub fn read_oid_sync(
        &self,
        key: &usize,
        guard: &I::Guard,
    ) -> Result<OidReadGuard<'_>, TransactionError> {
        let idx = self
            .index
            .get(key, guard)
            .ok_or(TransactionError::IndexNotFound)?;
        self.cache.read_lock_sync(idx)
    }

    pub async fn lock_rid_r(&self, rid: Rid) -> Result<OidReadGuard<'_>, TransactionError> {
        self.cache.read_lock(rid).await
    }

    pub async fn lock_rid_w(&self, rid: Rid) -> Result<OidWriteGuard<'_>, TransactionError> {
        self.cache.write_lock(rid).await
    }

    pub async fn write_oid(
        &self,
        key: &usize,
        guard: &I::Guard,
    ) -> Result<OidWriteGuard<'_>, TransactionError> {
        let idx = self
            .index
            .get(key, guard)
            .ok_or(TransactionError::IndexNotFound)?;
        self.cache.write_lock(idx).await
    }

    pub fn write_oid_sync(
        &self,
        key: &usize,
        guard: &I::Guard,
    ) -> Result<OidWriteGuard<'_>, TransactionError> {
        let idx = self
            .index
            .get(key, guard)
            .ok_or(TransactionError::IndexNotFound)?;
        self.cache.write_lock_sync(idx)
    }

    #[inline]
    pub async fn update<'a, const N: usize>(
        &'a self,
        write_oid: &mut OidWriteGuard<'a>,
        fields: UpdateQuery<'a, N>,
    ) {
        self.cache.update(write_oid, fields).await;
    }

    pub fn schema(&self) -> &S {
        self.cache.schema()
    }
}

const HUGEPAGE_SIZE: usize = 2 * 1024 * 1024;
struct PageBuffer<T: 'static> {
    next: AtomicUsize,
    data: *mut T,
}

unsafe impl<T> Send for PageBuffer<T> {}
unsafe impl<T> Sync for PageBuffer<T> {}

impl<T: 'static> PageBuffer<T> {
    const CAP: usize = HUGEPAGE_SIZE / std::mem::size_of::<T>();

    fn new() -> Self {
        let layout = Layout::from_size_align(HUGEPAGE_SIZE, HUGEPAGE_SIZE).unwrap();
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        let ptr = ptr as *mut T;
        Self {
            next: AtomicUsize::new(0),
            data: ptr,
        }
    }

    fn try_insert(&self, value: T) -> Result<(), T> {
        let next = self.next.load(Ordering::Relaxed);
        if next < Self::CAP {
            unsafe {
                let ptr = self.data.add(next);
                std::intrinsics::nontemporal_store(ptr, value);
            }
            self.next.fetch_add(1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(value)
        }
    }

    fn get_data_ptr(&self) -> *const T {
        self.data as *const T
    }

    fn reset_index(&self) {
        self.next.store(0, Ordering::Relaxed);
    }
}

impl<T> Drop for PageBuffer<T> {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(HUGEPAGE_SIZE, HUGEPAGE_SIZE).unwrap();
        unsafe {
            std::alloc::dealloc(self.data as *mut u8, layout);
        }
    }
}

/// This is a hack, ideally we should adapt our `DbTable` to support InsertOnlyTable.
/// Here we specialize it using a new type of table.
/// we only insert tuples to the table but never read back.
/// An example of such table is the `History` from TPCC
pub struct InsertonlyTable<T: 'static> {
    storage: ArrayAllocator<[usize; HUGEPAGE_SIZE / mem::size_of::<usize>()]>,
    tls_pages: HashMap<u64, PageBuffer<T>>,
}

impl<T: 'static> InsertonlyTable<T> {
    pub fn new(cap: usize) -> Self {
        let size = cap * mem::size_of::<T>();
        let size = align_up(size, HUGEPAGE_SIZE);

        let addr = PmHeap::get().alloc_frame(size);
        let pm_page_cnt = size / HUGEPAGE_SIZE;

        Self {
            storage: unsafe { ArrayAllocator::new(pm_page_cnt, addr) },
            tls_pages: HashMap::new(),
        }
    }

    /// for benchmark only, reset the start address, so we don't need to dealloc and alloc
    pub fn reset(&self) {
        self.tls_pages.pin().clear();
        unsafe {
            self.storage.reset();
        }
    }

    pub fn insert(&self, tuple: T) {
        let guard = flurry::epoch::pin();
        let page = self.get_tls_page(&guard);

        if let Err(tuple) = page.try_insert(tuple) {
            let pm_page = self.storage.alloc().unwrap().1;
            let pm_page = pm_page.as_mut_ptr() as *mut u8;
            let src_page_ptr = page.get_data_ptr() as *const u8;

            unsafe {
                nt_copy_nonoverlapping(src_page_ptr, pm_page, HUGEPAGE_SIZE);
                // std::ptr::copy_nonoverlapping(src_page_ptr, pm_page, HUGEPAGE_SIZE);
            }
            page.reset_index();
            let _ = page.try_insert(tuple);
        }
    }

    fn get_tls_page<'a>(&'a self, guard: &'a flurry::epoch::Guard) -> &'a PageBuffer<T> {
        let tid = get_tid();
        let tls_page = self.tls_pages.get(&tid, guard);
        match tls_page {
            Some(v) => v,
            None => {
                let page = PageBuffer::new();
                self.tls_pages.insert(tid, page, guard);
                self.get_tls_page(guard)
            }
        }
    }
}

impl<T> Drop for InsertonlyTable<T> {
    fn drop(&mut self) {
        let size = self.storage.capacity() * HUGEPAGE_SIZE;

        poison_memory_region(self.storage.start_addr() as *const u8, size);

        unsafe {
            PmHeap::get().dealloc_frame(self.storage.start_addr() as *mut u8, size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        async_task::block_on,
        index::FlurryMap,
        query::Field,
        utils::{
            test_gen::{FieldItemSchema, TestItem},
            TestGen,
        },
    };
    use std::{mem, slice};

    const FIELD_SZ: usize = 4;
    const TUPLE_SZ: usize = 16;

    const QUERY: FieldsMeta<4> = FieldsMeta::new([
        Field::new(0, 8),
        Field::new(24, 32),
        Field::new(72, 80),
        Field::new(80, 88),
    ]);

    type SchemaTy = FieldItemSchema<FIELD_SZ, TUPLE_SZ>;

    const TEST_SCHEMA: SchemaTy = FieldItemSchema::from_fields(QUERY.fields);

    fn value_compare<T, const N: usize>(
        values: &QueryValue<<SchemaTy as Schema>::Field, <SchemaTy as Schema>::Tuple, T>,
        query: &FieldsMeta<N>,
        tuple: &TestItem<TUPLE_SZ>,
    ) {
        for q in query.iter() {
            let gt_val: usize = unsafe { tuple.read_field(q) };
            let qy_val = unsafe { values.get_value::<usize, _>(q, &TEST_SCHEMA) };
            assert_eq!(gt_val, *qy_val);
        }
    }

    #[test]
    fn insert_read() {
        let record_cnt = 1000;
        let storage_size = record_cnt * mem::size_of::<TestItem<TUPLE_SZ>>();
        let cache_size = (record_cnt / 10) * mem::size_of::<TestItem<TUPLE_SZ>>();
        let table = DbTable::<FlurryMap, FieldItemSchema<FIELD_SZ, TUPLE_SZ>>::builder()
            .cache(cache_size)
            .probe_len(Some(4))
            .storage(storage_size)
            .schema(TEST_SCHEMA)
            .build();

        let guard = table.pin();
        for i in 0..record_cnt {
            let item = TestItem::from_increasing(i);
            let _x_guard = table.insert(item, &guard);
        }

        let query = TestItem::<TUPLE_SZ>::build_query([0, 3, 9]);
        for i in 0..record_cnt {
            let key = i;
            let oid_read = block_on(table.read_oid(&key, &guard)).unwrap();
            let value = table.read(&oid_read, &query);

            value_compare(&value, &query, &TestItem::<TUPLE_SZ>::from_increasing(i));
        }

        let query_miss = TestItem::<TUPLE_SZ>::build_query([1, 3, 10]);
        for i in 0..record_cnt {
            let key = i;
            let oid_read = block_on(table.read_oid(&key, &guard)).unwrap();
            let value = table.read(&oid_read, &query_miss);

            value_compare(
                &value,
                &query_miss,
                &TestItem::<TUPLE_SZ>::from_increasing(i),
            );
        }
    }

    #[test]
    fn update_read() {
        let record_cnt = 1000;
        let storage_size = record_cnt * mem::size_of::<TestItem<TUPLE_SZ>>();
        let cache_size = (record_cnt / 10) * mem::size_of::<TestItem<TUPLE_SZ>>();
        let table = DbTable::<FlurryMap, FieldItemSchema<FIELD_SZ, TUPLE_SZ>>::builder()
            .cache(cache_size)
            .probe_len(Some(4))
            .storage(storage_size)
            .schema(TEST_SCHEMA)
            .build();

        let guard = table.pin();
        for i in 0..record_cnt {
            let item = TestItem::from_increasing(i);
            let _x_guard = table.insert(item, &guard);
        }

        // Update cached fields
        let query = TestItem::<TUPLE_SZ>::build_query([0, 3, 9]);
        let query_data: [usize; 3] = [42, 42, 42];
        let update = UpdateQuery::from_meta(query, unsafe {
            slice::from_raw_parts(&query_data as *const usize as *const u8, 24)
        });

        for i in 0..record_cnt {
            let key = i;
            let mut write_oid = block_on(table.write_oid(&key, &guard)).unwrap();
            block_on(table.update(&mut write_oid, update.clone()));
        }

        // Read back updated entries
        let query = TestItem::<TUPLE_SZ>::build_query([0, 3, 9]);
        for i in 0..record_cnt {
            let key = i;
            let oid_read = block_on(table.read_oid(&key, &guard)).unwrap();
            let value = table.read(&oid_read, &query);

            for q in query.iter() {
                let qy_val = unsafe { value.get_value::<usize, _>(q, &TEST_SCHEMA) };
                assert_eq!(*qy_val, 42);
            }
        }

        // Update the uncached fields
        let query = TestItem::<TUPLE_SZ>::build_query([1, 3, 10]);
        let update_data: [usize; 3] = [24, 24, 24];
        let update = UpdateQuery::from_meta(query, unsafe {
            slice::from_raw_parts(&update_data as *const usize as *const u8, 24)
        });

        for i in 0..record_cnt {
            let key = i;
            let mut write_oid = block_on(table.write_oid(&key, &guard)).unwrap();
            block_on(table.update(&mut write_oid, update.clone()));
        }

        let query = TestItem::<TUPLE_SZ>::build_query([1, 3, 10]);
        for i in 0..record_cnt {
            let key = i;
            let oid_read = block_on(table.read_oid(&key, &guard)).unwrap();
            let value = table.read(&oid_read, &query);

            for q in query.iter() {
                let qy_val = unsafe { value.get_value::<usize, _>(q, &TEST_SCHEMA) };
                assert_eq!(*qy_val, 24);
            }
        }
    }

    use crossbeam_utils::thread::scope;
    use rand::Rng;

    #[test]
    fn multi_read() {
        let thread_cnt = 4;
        let record_per_thread = 1000;
        let record_cnt = record_per_thread * thread_cnt;
        let storage_size = record_cnt * mem::size_of::<TestItem<TUPLE_SZ>>();
        let cache_size = (record_cnt / 10) * mem::size_of::<TestItem<TUPLE_SZ>>();
        let table = DbTable::<FlurryMap, FieldItemSchema<FIELD_SZ, TUPLE_SZ>>::builder()
            .cache(cache_size)
            .probe_len(Some(4))
            .storage(storage_size)
            .schema(TEST_SCHEMA)
            .build();

        let guard = table.pin();
        for i in 0..record_cnt {
            let item = TestItem::from_increasing(i);
            let _x_guard = table.insert(item, &guard);
        }

        let multi_read = |query| {
            scope(|scope| {
                for _i in 0..thread_cnt {
                    scope.spawn(|_| {
                        let mut rng = rand::thread_rng();
                        let guard = table.pin();

                        for _r in 0..record_per_thread {
                            let key = rng.gen_range(0..record_cnt);
                            let oid_read = if let Ok(oid) = block_on(table.read_oid(&key, &guard)) {
                                oid
                            } else {
                                continue;
                            };
                            let value = table.read(&oid_read, &query);
                            value_compare(
                                &value,
                                &query,
                                &TestItem::<TUPLE_SZ>::from_increasing(key),
                            );
                        }
                    });
                }
            })
            .unwrap();
        };

        multi_read(TestItem::<TUPLE_SZ>::build_query([0, 3, 9]));
        multi_read(TestItem::<TUPLE_SZ>::build_query([0, 3, 10]));
    }
}
