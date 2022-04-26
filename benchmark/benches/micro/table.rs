use benchmark::{
    micro::{
        config::{self, bench_config::Table as TableConfig},
        Operation, RandDistribution, TransactionResult, WriteLogEntry,
    },
    CachePolicy,
};

use alchemy::{
    block_on,
    attribute_cache::Schema,
    index::{Art, DbIndex},
    log::{start_logger_thread, Logger},
    query::{Field, FieldsMeta},
    table::DbTable,
    utils::{
        test_gen::{FieldItemSchema, TestItem},
        TestGen,
    },
    Executor,
};
use metric::{timer, Timer};
use serde_json::json;
use shumai::{ShumaiBench, ShumaiResult};
use std::{
    ops::Range,
    sync::{atomic::AtomicUsize, Arc},
};

const FIELD_SCHEMA: FieldItemSchema<4, 16> = FieldItemSchema::from_fields([
    Field::new(0, 8),
    Field::new(24, 32),
    Field::new(72, 80),
    Field::new(80, 88),
]);

const TUPLE_SCHEMA: FieldItemSchema<16, 16> = FieldItemSchema::from_fields([
    Field::new(0, 8),
    Field::new(8, 16),
    Field::new(16, 24),
    Field::new(24, 32),
    Field::new(32, 40),
    Field::new(40, 48),
    Field::new(48, 56),
    Field::new(56, 64),
    Field::new(64, 72),
    Field::new(72, 80),
    Field::new(80, 88),
    Field::new(88, 96),
    Field::new(96, 104),
    Field::new(104, 112),
    Field::new(112, 120),
    Field::new(120, 128),
]);

fn item_cnt(storage_mb: usize) -> usize {
    storage_mb * MB / std::mem::size_of::<TestItem<16>>()
}

pub const MB: usize = 1024 * 1024;

const GROUP_SIZE: usize = 16;

pub struct MicroDB<I: DbIndex, S: Schema, const RN: usize, const WN: usize>
where
    S::Tuple: 'static,
{
    pub table: DbTable<I, S>,
    pub logger: Arc<Logger>,
    pub item_cnt: usize,
    pub read_query: FieldsMeta<RN>,
    pub update_query: FieldsMeta<WN>,
    pub config: TableConfig,
    future_size: AtomicUsize,
}

impl<I, S, const RN: usize, const WN: usize> ShumaiBench for MicroDB<I, S, RN, WN>
where
    I: DbIndex,
    S: Schema,
    S::Tuple: 'static + TestGen,
    S::Field: 'static,
{
    type Result = TransactionResult;
    type Config = TableConfig;

    fn run(&self, context: shumai::Context<Self::Config>) -> Self::Result {
        self.table.thread_init();

        metric::get_tls_recorder().reset();

        let mut result = match self.config.mem_engine {
            config::MemEngine::Async => self.async_bench(&context),
            config::MemEngine::Sync => self.sync_bench(&context),
        };

        result.set_metric(metric::get_tls_recorder().clone());
        result
    }

    fn load(&mut self) -> Option<serde_json::Value> {
        let load_thread = 1;
        let item_per_thread = self.item_cnt / load_thread;
        crossbeam_utils::thread::scope(|scope| {
            let handlers: Vec<_> = (0..load_thread)
                .map(|tid| {
                    let table_ref = &self.table;
                    scope.spawn(move |_| {
                        table_ref.thread_init();
                        let guard = table_ref.pin();
                        for i in tid * item_per_thread..(tid + 1) * item_per_thread {
                            let item = S::Tuple::from_increasing(i);
                            let _x_guard = table_ref.insert(item, &guard);
                        }
                    })
                })
                .collect();

            for h in handlers.into_iter() {
                h.join().unwrap();
            }
        })
        .unwrap();

        let logger = self.logger.clone();
        start_logger_thread(logger);

        let metric_enabled = cfg!(feature = "metrics");

        Some(json!({ "async_group_size": GROUP_SIZE , 
                    "log_entry_size": std::mem::size_of::<WriteLogEntry<WN>>(), 
                    "metrics_enabled": metric_enabled }))
    }

    fn cleanup(&mut self) -> Option<serde_json::Value> {
        self.logger.stop_logger();
        Some(json!({"future_size": self.future_size.load(std::sync::atomic::Ordering::Relaxed)}))
    }
}

impl<I, S, const RN: usize, const WN: usize> MicroDB<I, S, RN, WN>
where
    I: DbIndex,
    S: Schema,
    S::Tuple: 'static + TestGen,
    S::Field: 'static,
{
    pub fn new(
        table: DbTable<I, S>,
        item_cnt: usize,
        read_query: FieldsMeta<RN>,
        update_query: FieldsMeta<WN>,
        config: TableConfig,
    ) -> Self {
        Self {
            table,
            logger: Arc::new(Logger::new()),
            item_cnt,
            read_query,
            update_query,
            config,
            future_size: AtomicUsize::new(0),
        }
    }

    pub fn sync_bench(&self, context: &shumai::Context<TableConfig>) -> TransactionResult {
        context.wait_for_start();

        let item_per_task = self.item_cnt / context.thread_cnt();
        let tid = context.thread_id() as usize;

        block_on(self.bench_task(context, item_per_task * tid..item_per_task * (tid + 1)))
    }

    pub fn async_bench(&self, context: &shumai::Context<TableConfig>) -> TransactionResult {
        if context.thread_id() == 0 {
            let future = self.bench_task(&context, 0..0);
            self.future_size.store(
                std::mem::size_of_val(&future),
                std::sync::atomic::Ordering::Relaxed,
            );
        }

        let total_task = context.thread_cnt() * GROUP_SIZE;
        let item_per_task = self.item_cnt / total_task;

        let mut executor = Executor::<_, GROUP_SIZE>::new();
        for i in 0..GROUP_SIZE {
            let task_id = context.thread_id() as usize * GROUP_SIZE + i;
            let item_range = task_id * item_per_task..(task_id + 1) * item_per_task;

            executor.spawn(self.bench_task(&context, item_range));
        }

        context.wait_for_start();

        executor.run_ready_tasks().iter().sum()
    }

    async fn bench_task(
        &self,
        context: &shumai::Context<'_, TableConfig>,
        item_range: Range<usize>,
    ) -> TransactionResult {
        let mut rng = rand::thread_rng();
        let mut result = TransactionResult::default();

        let dist = RandDistribution::from(self.config.distribution, item_range);

        let guard = self.table.pin();
        let workload = self.config.workload;

        const OP_PER_TXN: usize = 10;

        let mut log_entries = vec![WriteLogEntry::default(); OP_PER_TXN];

        loop {
            timer!(Timer::Read);
            let mut log_idx = 0;

            let _lsn = self.logger.acquire_lsn();

            for _i in 0..OP_PER_TXN {
                let key = dist.sample(&mut rng) as usize;

                match Operation::gen(&mut rng, workload) {
                    Operation::Read => {
                        let write_oid = self.table.write_oid(&key, &guard).await;

                        match write_oid {
                            Ok(mut write_oid) => {
                                let rv = self
                                    .table
                                    .read_and_promote(&mut write_oid, &self.read_query)
                                    .await;
                                rv.prefetch_all().await;
                                unsafe {
                                    for q in self.read_query.iter() {
                                        let _v = *rv.get_value::<usize, _>(q, self.table.schema());
                                    }
                                }
                            }
                            Err(_) => result.inc_abort(),
                        };
                    }
                    Operation::Update => {
                        let update_values = [result.commit; WN];
                        log_entries[log_idx] = WriteLogEntry::create_write(key, update_values);
                        log_idx += 1;

                        let write_oid = self.table.write_oid(&key, &guard).await;

                        match write_oid {
                            Ok(mut write_oid) => {
                                let rv = self
                                    .table
                                    .read_and_promote(&mut write_oid, &self.read_query)
                                    .await;
                                rv.prefetch_all().await;
                                for (i, q) in self.update_query.iter().enumerate() {
                                    let v: &mut usize =
                                        unsafe { rv.get_value_mut(q, self.table.schema()) };
                                    *v = update_values[i];
                                }
                            }
                            Err(_) => result.inc_abort(),
                        };
                    }
                    Operation::Insert => {
                        let item = S::Tuple::from_increasing(key);
                        let _x_guard = self.table.insert(item, &guard);
                    }
                };
            }

            if log_idx > 0 {
                let log_size = log_idx * std::mem::size_of::<WriteLogEntry<WN>>();
                let buffer = self.logger.acquire_log_buffer(log_size);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        log_entries.as_ptr() as *const u8,
                        buffer as *mut [u8] as *mut u8,
                        log_size,
                    );
                }
            }

            result.inc_commit();

            if !context.is_running() {
                return result;
            }
        }
    }
}

fn run_bench<I, const N: usize>(
    c: &TableConfig,
    table: DbTable<I, FieldItemSchema<N, 16>>,
) -> ShumaiResult<TableConfig, TransactionResult>
where
    I: DbIndex + 'static,
{
    let item_cnt = item_cnt(c.data_size_mb);
    let read_query = TestItem::<16>::build_query([0, 3, 9, 10]);
    let update_query = TestItem::<16>::build_query([0, 3, 9, 10]);

    let mut table_bench = MicroDB::new(table, item_cnt, read_query, update_query, c.clone());
    shumai::run(&mut table_bench, c, 6)
}

fn main() {
    let configs = TableConfig::load().expect("no table config found!");

    for c in configs {
        // A lot larger than needed storage size to support insert operations
        let extra_storage_size = c.data_size_mb * MB * 1;

        let results = match c.policy {
            CachePolicy::Field => {
                let table = DbTable::<Art, FieldItemSchema<4, 16>>::builder()
                    .cache(c.cache_size_mb * MB)
                    .probe_len(Some(c.probe_len))
                    .probe_rng(c.probe_rng)
                    .storage(extra_storage_size)
                    .schema(FIELD_SCHEMA)
                    .build();
                run_bench(&c, table)
            }
            CachePolicy::Tuple => {
                let table = DbTable::<Art, FieldItemSchema<16, 16>>::builder()
                    .cache(c.cache_size_mb * MB)
                    .probe_len(Some(c.probe_len))
                    .probe_rng(c.probe_rng)
                    .storage(extra_storage_size)
                    .schema(TUPLE_SCHEMA)
                    .build();
                run_bench(&c, table)
            }
            CachePolicy::Direct => {
                let table = DbTable::<Art, FieldItemSchema<4, 16>>::builder()
                    .cache(c.cache_size_mb * MB)
                    .probe_len(Some(0))
                    .probe_rng(0.0)
                    .storage(extra_storage_size)
                    .schema(FIELD_SCHEMA)
                    .build();
                run_bench(&c, table)
            }
        };

        results.write_json().unwrap();
    }
}
