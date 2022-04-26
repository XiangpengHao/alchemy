use crate::cache_manager::Schema;
use crate::utils::test_gen::{value_compare, FieldItemSchema, TestItem};
use crate::{
    async_task::block_on,
    cache_manager::Rid,
    query::{Field, FieldsMeta},
    utils::TestGen,
};
use std::sync::atomic::{AtomicUsize, Ordering};

const QUERY: FieldsMeta<4> = FieldsMeta::new([
    Field::new(0, 8),
    Field::new(24, 32),
    Field::new(72, 80),
    Field::new(80, 88),
]);

const TUPLE_SZ: usize = 16;
const FIELD_SZ: usize = 4;
type CacheSchema = FieldItemSchema<FIELD_SZ, TUPLE_SZ>;

const TEST_SCHEMA: CacheSchema = FieldItemSchema::from_fields(QUERY.fields);

const PROBE_LEN: usize = 16;

#[test]
fn clock_insert() {
    let capacity = PROBE_LEN * 8;
    let item_cnt = capacity * 10;

    let (cache, rid_array) = gen_cache_and_oid(capacity, item_cnt);

    // Read the first few items without touching the cache manager
    // the first few items should be pointers
    for (i, rid) in rid_array.iter().enumerate().take(item_cnt - capacity) {
        let cur_oid = block_on(cache.inner.oid_array.get(*rid)).read();
        if cur_oid.is_rid() {
            let p = cur_oid.to_rid();
            let storage_p = unsafe { &*cache.inner.storage.get(p).get() };
            let item = storage_p.clone();
            assert_eq!(item, TestItem::from_increasing(i));
        } else {
            unreachable!();
        }
    }

    // Reference the last few items so their hotness are incremented
    // The last few items should hit cache
    for (i, rid) in rid_array.iter().enumerate().skip(item_cnt - capacity) {
        let mut locked_oid = block_on(cache.inner.oid_array.get(*rid)).write();
        let qy_rt = block_on(cache.read_and_promote(&mut locked_oid, &QUERY));
        assert!(qy_rt.cached_hit());
        assert!(!qy_rt.has_tuple());
        assert!(value_compare(
            &qy_rt,
            &QUERY,
            &TestItem::<16>::from_increasing(i),
            &TEST_SCHEMA,
        ));
    }

    // Reference the first few items so all hotness are reset
    for (i, rid) in rid_array
        .iter()
        .enumerate()
        .take(capacity / cache.inner.probe_len())
    {
        let mut locked_oid = cache.inner.oid_array.get_sync(*rid).write();
        let val = block_on(cache.read_and_promote(&mut locked_oid, &QUERY));

        // The return value should be newly promoted to cache
        assert!(!val.cached_hit());
        assert!(value_compare(
            &val,
            &QUERY,
            &TestItem::<16>::from_increasing(i),
            &TEST_SCHEMA,
        ));
    }

    // Now flooding the cache, every one should be a cache miss and be promoted to cache
    for (i, rid) in rid_array.iter().enumerate().take(capacity) {
        let mut locked_oid = cache.inner.oid_array.get_sync(*rid).write();
        let val = block_on(cache.read_and_promote(&mut locked_oid, &QUERY));

        // The return value should be newly promoted to cache
        assert!(val.cached_hit());
        assert!(value_compare(
            &val,
            &QUERY,
            &TestItem::<16>::from_increasing(i),
            &TEST_SCHEMA,
        ));
    }
}

use crossbeam_utils::thread::scope;
use rand::Rng;

#[test]
fn clock_multi_read() {
    let item_cnt = 1000;
    let capacity = 100;

    let (cache, rid_array) = gen_cache_and_oid(capacity, item_cnt);

    const THREADS: usize = 8;
    const READ_CNT: usize = 10000;
    scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|_| {
                let mut rng = rand::thread_rng();
                for _i in 0..READ_CNT {
                    let idx = rng.gen_range(0..item_cnt);
                    let rid = rid_array[idx];
                    let mut locked_oid = cache.inner.oid_array.get_sync(rid).write();
                    let val = block_on(cache.read_and_promote(&mut locked_oid, &QUERY));
                    assert!(value_compare(
                        &val,
                        &QUERY,
                        &TestItem::<16>::from_increasing(idx),
                        &TEST_SCHEMA,
                    ));
                }
            });
        }
    })
    .unwrap();
}

use crossbeam_utils::Backoff;
use std::sync::Arc;

use super::cache_inner::ClockNode;
use super::oid::OidGuard;
use super::ClockCache;

#[test]
fn clock_multi_insert() {
    const THREADS: usize = 8;
    const ITEM_PER_THREAD: usize = 1000;

    let capacity = 100;

    let (cache, rid_array) = gen_cache_and_oid(capacity, ITEM_PER_THREAD * THREADS);

    let remaining = Arc::new(AtomicUsize::new(THREADS));
    scope(|scope| {
        for _tid in 0..THREADS {
            scope.spawn(|_| {
                let rm = remaining.clone();

                for (i, item) in rid_array.iter().enumerate().take(ITEM_PER_THREAD) {
                    let mut locked_oid = cache.inner.oid_array.get_sync(*item).write();
                    let val = block_on(cache.read_and_promote(&mut locked_oid, &QUERY));
                    assert!(value_compare(
                        &val,
                        &QUERY,
                        &TestItem::<16>::from_increasing(i),
                        &TEST_SCHEMA,
                    ));
                }

                // To prevent the threads from exiting too early,
                // while other threads still holding the reference to oid_array
                // This bug was caught by addr sanitizer
                rm.fetch_sub(1, Ordering::Relaxed);
                let backoff = Backoff::new();
                loop {
                    if rm.load(Ordering::Relaxed) == 0 {
                        break;
                    }
                    backoff.snooze();
                }
            });
        }
    })
    .unwrap();
}

fn gen_cache_and_oid(cache_cnt: usize, tuple_cnt: usize) -> (ClockCache<CacheSchema>, Vec<Rid>) {
    let cache_size = cache_cnt * std::mem::size_of::<ClockNode<<CacheSchema as Schema>::Field>>();
    let storage_size = tuple_cnt * std::mem::size_of::<<CacheSchema as Schema>::Tuple>();
    let cache =
        ClockCache::<CacheSchema>::new(cache_size, PROBE_LEN, 1.0, storage_size, TEST_SCHEMA, None);
    let mut rid_array = Vec::with_capacity(tuple_cnt);
    for i in 0..tuple_cnt {
        let item = TestItem::from_increasing(i);
        let (rid, _) = cache.insert(item);
        rid_array.push(rid);
    }
    (cache, rid_array)
}
