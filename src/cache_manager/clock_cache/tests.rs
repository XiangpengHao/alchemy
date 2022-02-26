use crate::utils::test_gen::{value_compare, FieldItemSchema, TestItem};
use crate::{
    async_task::block_on,
    cache_manager::{Rid, Tuple},
    query::{Field, FieldsMeta, UpdateQuery},
    utils::TestGen,
};
use std::{
    slice,
    sync::atomic::{AtomicUsize, Ordering},
};

const QUERY: FieldsMeta<4> = FieldsMeta::new([
    Field::new(0, 8),
    Field::new(24, 32),
    Field::new(72, 80),
    Field::new(80, 88),
]);
type CacheSchema = FieldItemSchema<4, 16>;
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
        let cur_oid = block_on(cache.oid_array.get(*rid)).read();
        if cur_oid.is_rid() {
            let p = cur_oid.to_rid();
            let storage_p = unsafe { &*cache.storage().get(p).get() };
            let item = storage_p.clone();
            assert_eq!(item, TestItem::from_increasing(i));
        } else {
            unreachable!();
        }
    }

    // Reference the last few items so their hotness are incremented
    // The last few items should hit cache
    for (i, rid) in rid_array.iter().enumerate().skip(item_cnt - capacity) {
        let mut locked_oid = block_on(cache.oid_array.get(*rid)).write();
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
        .take(capacity / cache.probe_len())
    {
        let mut locked_oid = cache.oid_array.get_sync(*rid).write();
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
        let mut locked_oid = cache.oid_array.get_sync(*rid).write();
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

    let cache = CacheInner::<FieldItemSchema<FIELD_SZ, TUPLE_SZ>>::new_cap(
        capacity,
        PROBE_LEN,
        0.1,
        item_cnt,
        0,
        TEST_SCHEMA,
        None,
    );
    let mut rid_array = Vec::new();
    for i in 0..item_cnt {
        let (rid, _write_lock) = cache.oid_array.alloc_rid();
        let item = TestItem::from_increasing(i);
        cache.storage().insert(&rid, item);
        rid_array.push(rid);
    }

    const THREADS: usize = 8;
    const READ_CNT: usize = 10000;
    scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|_| {
                let mut rng = rand::thread_rng();
                for _i in 0..READ_CNT {
                    let idx = rng.gen_range(0..item_cnt);
                    let rid = rid_array[idx];
                    let mut locked_oid = cache.oid_array.get_sync(rid).write();
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

use super::{cache_inner::CacheInner, oid::OidGuard};

const TUPLE_SZ: usize = 16;
const FIELD_SZ: usize = 4;

#[test]
fn clock_multi_insert() {
    const THREADS: usize = 8;
    const ITEM_PER_THREAD: usize = 1000;

    let capacity = 100;
    let cache = CacheInner::<FieldItemSchema<FIELD_SZ, TUPLE_SZ>>::new_cap(
        capacity,
        PROBE_LEN,
        0.1,
        THREADS * ITEM_PER_THREAD,
        0,
        TEST_SCHEMA,
        None,
    );

    let remaining = Arc::new(AtomicUsize::new(THREADS));
    scope(|scope| {
        for _tid in 0..THREADS {
            scope.spawn(|_| {
                let mut rid_array = Vec::new();
                let rm = remaining.clone();
                for i in 0..ITEM_PER_THREAD {
                    let (rid, mut write_lock) = cache.oid_array.alloc_rid();
                    let item = TestItem::from_increasing(i);
                    cache.insert(&mut write_lock, item);
                    rid_array.push(rid);
                }

                for (i, item) in rid_array.iter().enumerate().take(ITEM_PER_THREAD) {
                    let mut locked_oid = cache.oid_array.get_sync(*item).write();
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

fn gen_cache_and_oid(
    cache_cnt: usize,
    tuple_cnt: usize,
) -> (CacheInner<FieldItemSchema<4, 16>>, Vec<Rid>) {
    let cache = CacheInner::<FieldItemSchema<FIELD_SZ, TUPLE_SZ>>::new_cap(
        cache_cnt,
        PROBE_LEN,
        1.0,
        tuple_cnt,
        0,
        TEST_SCHEMA,
        None,
    );
    let mut rid_array = Vec::with_capacity(tuple_cnt);
    for i in 0..tuple_cnt {
        let (rid, mut write_lock) = cache.oid_array.alloc_rid();
        rid_array.push(rid);
        let item = TestItem::from_increasing(i);
        cache.insert(&mut write_lock, item);
    }
    (cache, rid_array)
}

#[test]
fn clock_update() {
    let capacity = PROBE_LEN * 8;
    let item_cnt = capacity * 10;

    let (cache, rid_array) = gen_cache_and_oid(capacity, item_cnt);

    // Step 1: go through every item to ensure the values are original ones
    for (idx, rid) in rid_array.iter().enumerate() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        let rv = block_on(cache.read_and_promote(&mut cur_oid, &QUERY));
        assert!(value_compare(
            &rv,
            &QUERY,
            &TestItem::<16>::from_increasing(idx),
            &TEST_SCHEMA,
        ));
    }

    // Step 2: update every cached items (with 100% schema hit)
    let fields = TestItem::<TUPLE_SZ>::build_query([0, 3, 9]);
    let update_data: [usize; 3] = [42, 42, 42];

    let update_fields = UpdateQuery::from_meta(fields, unsafe {
        slice::from_raw_parts(&update_data as *const usize as *const u8, 24)
    });

    for rid in rid_array.iter() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        block_on(cache.update(&mut cur_oid, update_fields.clone()));
    }

    // Step 3: read back every cached item to ensure values are updated
    for (idx, rid) in rid_array.iter().enumerate() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        let rv = block_on(cache.read_and_promote(&mut cur_oid, &QUERY));
        let mut updated = TestItem::<16>::from_increasing(idx);
        updated.update(update_fields.clone());

        assert!(value_compare(&rv, &QUERY, &updated, &TEST_SCHEMA));
    }

    // Step 4: update fields again but the schema will miss
    let update_data: [usize; 4] = [24, 24, 24, 24];
    let update_fields = UpdateQuery::from_meta(QUERY, unsafe {
        slice::from_raw_parts(&update_data as *const usize as *const u8, 32)
    });

    for rid in rid_array.iter() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        block_on(cache.update(&mut cur_oid, update_fields.clone()));
    }

    // Step 5: read back every item to ensure values are updated
    for (idx, rid) in rid_array.iter().enumerate() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        let rv = block_on(cache.read_and_promote(&mut cur_oid, &QUERY));
        let mut updated = TestItem::<16>::from_increasing(idx);
        updated.update(update_fields.clone());

        assert!(value_compare(&rv, &QUERY, &updated, &TEST_SCHEMA));
    }
}

#[test]
fn clock_update_schema_miss() {
    let capacity = PROBE_LEN * 8;
    let item_cnt = capacity;

    const QUERY_MISS: FieldsMeta<3> =
        FieldsMeta::new([Field::new(8, 16), Field::new(24, 32), Field::new(72, 80)]);

    let (cache, rid_array) = gen_cache_and_oid(capacity, item_cnt);

    // Step 1: go through every item to ensure the values are original ones
    for (idx, rid) in rid_array.iter().enumerate() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        let rv = block_on(cache.read_and_promote(&mut cur_oid, &QUERY));
        assert!(value_compare(
            &rv,
            &QUERY,
            &TestItem::<16>::from_increasing(idx),
            &TEST_SCHEMA,
        ));
    }

    // Step 2: update every cached items (with 100% schema hit)
    let fields = TestItem::<TUPLE_SZ>::build_query([0, 3, 9]);
    let update_data: [usize; 3] = [42, 42, 42];

    let update_fields = UpdateQuery::from_meta(fields, unsafe {
        slice::from_raw_parts(&update_data as *const usize as *const u8, 24)
    });

    for rid in rid_array.iter() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        block_on(cache.update(&mut cur_oid, update_fields.clone()));
    }

    // Step 3: read back schema miss item to ensure values are updated
    for (_idx, rid) in rid_array.iter().enumerate() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        let rv = block_on(cache.read_and_promote(&mut cur_oid, &QUERY_MISS));

        let val = unsafe { rv.get_value::<usize, CacheSchema>(&Field::new(24, 32), &TEST_SCHEMA) };
        assert_eq!(*val, 42);

        let val = unsafe { rv.get_value::<usize, CacheSchema>(&Field::new(72, 80), &TEST_SCHEMA) };
        assert_eq!(*val, 42);
    }
}

#[test]
fn clock_multi_update() {
    let capacity = PROBE_LEN * 8;
    let item_cnt = capacity * 50;

    let (cache, rid_array) = gen_cache_and_oid(capacity, item_cnt);

    // Step 1: go through every item to ensure the values are original ones
    for (idx, rid) in rid_array.iter().enumerate() {
        let mut cur_oid = cache.oid_array.get_sync(*rid).write();
        let rv = block_on(cache.read_and_promote(&mut cur_oid, &QUERY));
        assert!(value_compare(
            &rv,
            &QUERY,
            &TestItem::<16>::from_increasing(idx),
            &TEST_SCHEMA,
        ));
    }

    let threads = 4;
    let operation_per_thread = 100;
    let hit_data: [usize; 3] = [42, 42, 42];
    let hit_fields = UpdateQuery::new(
        [Field::new(0, 8), Field::new(24, 32), Field::new(72, 80)],
        unsafe { slice::from_raw_parts(&hit_data as *const usize as *const u8, 32) },
    );

    let miss_data: [usize; 4] = [42, 42, 42, 42];
    let miss_fields = UpdateQuery::from_meta(QUERY, unsafe {
        slice::from_raw_parts(&miss_data as *const usize as *const u8, 32)
    });

    scope(|scope| {
        for _tid in 0..threads / 2 {
            scope.spawn(|_| {
                let mut rng = rand::thread_rng();
                for _i in 0..operation_per_thread {
                    let idx = rng.gen_range(0..item_cnt);
                    let rid = rid_array[idx];
                    let mut locked_oid = cache.oid_array.get_sync(rid).write();
                    block_on(cache.update(&mut locked_oid, hit_fields.clone()));
                }
            });
        }
        for _tid in 0..threads / 2 {
            scope.spawn(|_| {
                let mut rng = rand::thread_rng();
                for _i in 0..operation_per_thread {
                    let idx = rng.gen_range(0..item_cnt);
                    let rid = rid_array[idx];
                    let mut locked_oid = cache.oid_array.get_sync(rid).write();
                    block_on(cache.update(&mut locked_oid, miss_fields.clone()));
                }
            });
        }
    })
    .unwrap();

    scope(|scope| {
        for _tid in 0..threads {
            scope.spawn(|_| {
                let mut rng = rand::thread_rng();
                for _i in 0..operation_per_thread {
                    let idx = rng.gen_range(0..item_cnt);
                    let rid = rid_array[idx];
                    let mut locked_oid = cache.oid_array.get_sync(rid).write();
                    let val = block_on(cache.read_and_promote(&mut locked_oid, &QUERY));

                    // Values can be the combination of three different
                    let origin = TestItem::<16>::from_increasing(idx);
                    let mut update_hit = origin.clone();
                    update_hit.update(hit_fields.clone());
                    let mut update_miss = origin.clone();
                    update_miss.update(miss_fields.clone());

                    let possible_values = [origin, update_hit, update_miss];

                    assert!(possible_values.iter().any(|x| value_compare(
                        &val,
                        &QUERY,
                        x,
                        &TEST_SCHEMA
                    )));
                }
            });
        }
    })
    .unwrap();
}
