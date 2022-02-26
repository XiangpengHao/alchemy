use benchmark::{
    tpcc::{
        config::{self, bench_config::Tpcc as TpccConfig, Workload},
        query::{
            delivery::DeliveryInput, new_order::NewOrderInput, order_status::OrderStatusInput,
            payment::PaymentInput, stock_level::StockLevelInput,
        },
        schemas::{CustomerCachedRead, GeneralSchema, ItemCached, OrderCached, StockCached},
        tables::{
            CustomerTuple, DistrictTuple, HistoryTuple, ItemTuple, OrderLineTuple, OrderTuple,
            StockTuple, WarehouseTuple,
        },
        workload::TpccWorkload,
        TpccResults,
    },
    utils::txn_err_to_counter,
    CachePolicy,
};
use std::{
    mem,
    sync::{atomic::AtomicUsize, Arc},
};

use alchemy::{block_on, cache_manager::Schema, error::TransactionError, log::Logger, Executor};

use metric::{counter, timer};
use serde_json::{json, Value};

use shumai::{Context, ShumaiBench};

const GROUP_SIZE: usize = 4;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

pub struct TpccBench<W, D, C, I, S, O, OL>
where
    GeneralSchema<W>: Schema<Tuple = WarehouseTuple>,
    GeneralSchema<D>: Schema<Tuple = DistrictTuple>,
    GeneralSchema<C>: Schema<Tuple = CustomerTuple>,
    GeneralSchema<I>: Schema<Tuple = ItemTuple>,
    GeneralSchema<S>: Schema<Tuple = StockTuple>,
    GeneralSchema<O>: Schema<Tuple = OrderTuple>,
    GeneralSchema<OL>: Schema<Tuple = OrderLineTuple>,
{
    warehouse_cnt: usize,
    workload: TpccWorkload<W, D, C, I, S, O, OL>,
    config: TpccConfig,
    logger: Arc<Logger>,
    gen_size: AtomicUsize,
}

impl<W, D, C, I, S, O, OL> TpccBench<W, D, C, I, S, O, OL>
where
    GeneralSchema<W>: Schema<Tuple = WarehouseTuple>,
    GeneralSchema<D>: Schema<Tuple = DistrictTuple>,
    GeneralSchema<C>: Schema<Tuple = CustomerTuple>,
    GeneralSchema<I>: Schema<Tuple = ItemTuple>,
    GeneralSchema<S>: Schema<Tuple = StockTuple>,
    GeneralSchema<O>: Schema<Tuple = OrderTuple>,
    GeneralSchema<OL>: Schema<Tuple = OrderLineTuple>,
{
    fn new(
        warehouse_cnt: usize,
        workload: TpccWorkload<W, D, C, I, S, O, OL>,
        config: TpccConfig,
    ) -> Self {
        Self {
            warehouse_cnt,
            workload,
            config,
            logger: Arc::new(Logger::new()),
            gen_size: AtomicUsize::new(0),
        }
    }

    fn sync_bench(&self, context: Context<TpccConfig>) -> TpccResults {
        context.wait_for_start();
        block_on(self.bench_task(&context, context.thread_id(), context.thread_cnt()))
    }

    fn async_bench(&self, context: Context<TpccConfig>) -> TpccResults {
        let mut executor = Executor::<_, GROUP_SIZE>::new();

        let group_size = GROUP_SIZE;
        let task_cnt = group_size * context.thread_cnt();
        for i in 0..group_size {
            let task_id = context.thread_id() * group_size + i;
            executor.spawn(self.bench_task(&context, task_id, task_cnt));
        }

        if context.thread_id() == 0 {
            let future = self.bench_task(&context, 0, task_cnt);
            self.gen_size.store(
                std::mem::size_of_val(&future),
                std::sync::atomic::Ordering::Relaxed,
            );
        }

        context.wait_for_start();

        executor.run_ready_tasks().iter().sum()
    }

    async fn bench_task(
        &self,
        context: &Context<'_, TpccConfig>,
        task_id: usize,
        task_cnt: usize,
    ) -> TpccResults {
        let mut txn_result = TpccResults::default();
        let mut rng = rand::thread_rng();
        loop {
            let workload = self.config.workload.gen(&mut rng);
            let _lsn = self.logger.acquire_lsn();
            match workload {
                Workload::Payment => {
                    let mut retry_cnt = 0;
                    let input = PaymentInput::gen(self.warehouse_cnt, task_id, task_cnt);
                    while let Err(e) = self.workload.payment(&input).await {
                        counter!(txn_err_to_counter(&e));
                        txn_result.payment.inc_abort();

                        retry_cnt += 1;
                        if retry_cnt >= 5 {
                            // deadlock prevention on async txn
                            break;
                        }
                    }

                    self.log_txn(input);

                    txn_result.payment.inc_commit();
                }
                Workload::NewOrder => {
                    timer!(metric::Timer::Tpcc);
                    let mut retry_cnt = 0;
                    let input = NewOrderInput::gen(self.warehouse_cnt, task_id, task_cnt);
                    while let Err(e) = self.workload.new_order(&input).await {
                        txn_result.new_order.inc_abort();

                        counter!(txn_err_to_counter(&e));

                        retry_cnt += 1;
                        if retry_cnt >= 5 {
                            // deadlock prevention on async txn
                            break;
                        }
                    }
                    self.log_txn(input);
                    txn_result.new_order.inc_commit();
                }
                Workload::OrderStatus => {
                    let mut retry_cnt = 0;
                    let input = OrderStatusInput::gen(self.warehouse_cnt, task_id, task_cnt);
                    while let Err(e) = self.workload.order_status(&input).await {
                        txn_result.order_status.inc_abort();

                        counter!(txn_err_to_counter(&e));

                        retry_cnt += 1;
                        if retry_cnt >= 5 {
                            break;
                        }
                    }
                    // order status is read only, no logging needed
                    txn_result.order_status.inc_commit();
                }
                Workload::Delivery => {
                    let mut retry_cnt = 0;
                    let input = DeliveryInput::gen(self.warehouse_cnt, task_id, task_cnt);
                    while let Err(e) = self.workload.delivery(&input).await {
                        txn_result.delivery.inc_abort();

                        counter!(txn_err_to_counter(&e));

                        retry_cnt += 1;
                        if retry_cnt >= 5 {
                            break;
                        }
                    }
                    self.log_txn(input);
                    txn_result.delivery.inc_commit();
                }
                Workload::StockLevel => {
                    let mut retry_cnt = 0;
                    let input = StockLevelInput::gen(self.warehouse_cnt, task_id, task_cnt);
                    while let Err(e) = self.workload.stock_level(&input).await {
                        txn_result.stock_level.inc_abort();

                        counter!(txn_err_to_counter(&e));

                        retry_cnt += 1;
                        if retry_cnt >= 5 {
                            break;
                        }
                    }
                    self.log_txn(input);
                    txn_result.stock_level.inc_commit();
                }
            };

            if !context.is_running() {
                return txn_result;
            }
        }
    }

    fn log_txn<E: bincode::enc::Encode>(&self, log_entry: E) {
        let entry_size = mem::size_of::<E>();
        let buffer = self.logger.acquire_log_buffer(entry_size);
        let _rv =
            bincode::encode_into_slice(log_entry, buffer, bincode::config::standard()).unwrap();
    }

    fn additional_stats(&self) -> Value {
        let stats = json!({
            "w_size": mem::size_of::<WarehouseTuple>(),
            "w_cache_size": mem::size_of::<W>(),
            "w_cache_align": mem::align_of::<W>(),

            "d_size": mem::size_of::<DistrictTuple>(),
            "d_cache_size": mem::size_of::<D>(),
            "d_cache_align": mem::align_of::<D>(),

            "c_size": mem::size_of::<CustomerTuple>(),
            "c_cache_size": mem::size_of::<C>(),
            "c_cache_align": mem::align_of::<C>(),

            "s_size": mem::size_of::<StockTuple>(),
            "s_cache_size": mem::size_of::<S>(),
            "s_cache_align": mem::align_of::<S>(),

            "i_size": mem::size_of::<ItemTuple>(),
            "i_cache_size": mem::size_of::<I>(),
            "i_cache_align": mem::align_of::<I>(),

            "h_size": mem::size_of::<HistoryTuple>(),
            "ol_size": mem::size_of::<OrderLineTuple>(),
            "o_size": mem::size_of::<OrderTuple>(),
        });

        stats
    }
}

impl<W, D, C, I, S, O, OL> ShumaiBench for TpccBench<W, D, C, I, S, O, OL>
where
    GeneralSchema<W>: Schema<Tuple = WarehouseTuple>,
    GeneralSchema<D>: Schema<Tuple = DistrictTuple>,
    GeneralSchema<C>: Schema<Tuple = CustomerTuple>,
    GeneralSchema<I>: Schema<Tuple = ItemTuple>,
    GeneralSchema<S>: Schema<Tuple = StockTuple>,
    GeneralSchema<O>: Schema<Tuple = OrderTuple>,
    GeneralSchema<OL>: Schema<Tuple = OrderLineTuple>,
{
    type Result = TpccResults;
    type Config = TpccConfig;

    fn run(&self, context: Context<Self::Config>) -> Self::Result {
        self.workload.thread_init();

        metric::get_tls_recorder().reset();

        let mut result = match context.config.txn_engine {
            config::TxnEngine::Async => self.async_bench(context),
            config::TxnEngine::Sync => self.sync_bench(context),
        };

        let metric_val = metric::get_tls_recorder().clone();
        result.set_metric(metric_val);

        result
    }

    fn load(&mut self) -> Option<serde_json::Value> {
        self.workload.load();

        let logger = self.logger.clone();
        alchemy::log::start_logger_thread(logger);
        Some(self.additional_stats())
    }

    fn on_thread_finished(&mut self, cur_thread: usize) {
        let mut config_iter = self.config.threads.iter();
        config_iter.position(|t| *t == cur_thread).unwrap();

        if let Some(thread_cnt) = config_iter.next() {
            self.workload.reset_order(*thread_cnt);
        }
    }

    fn cleanup(&mut self) -> Option<serde_json::Value> {
        self.logger.stop_logger();
        None
    }
}

pub fn register_panic_handler() {
    let default_panic = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |panic_info| {
        default_panic(panic_info);

        // Don't forget to enable core dumps on your shell with eg `ulimit -c unlimited`
        let pid = std::process::id();
        eprintln!("dumping core for pid {}", std::process::id());

        unsafe { libc::kill(pid.try_into().unwrap(), libc::SIGQUIT) };
    }));
}

fn main() -> Result<(), TransactionError> {
    let filter = std::env::args().nth(1).unwrap_or_else(|| ".*".to_string());
    let configs = TpccConfig::load_with_filter("tpcc.toml", filter).expect("no config found!");

    // register_panic_handler();

    let repeat = 5;

    for c in configs {
        let warehouse_cnt = c.warehouse_cnt;
        let customer_cache_mb = 4 * warehouse_cnt as usize;
        let stock_cache_mb = 4 * warehouse_cnt as usize;

        let results = match c.policy {
            CachePolicy::Field => {
                let mut payment_bench = TpccBench::<
                    WarehouseTuple,
                    DistrictTuple,
                    CustomerCachedRead,
                    ItemCached,
                    StockCached,
                    OrderCached,
                    OrderLineTuple,
                >::new(
                    warehouse_cnt,
                    TpccWorkload::new(
                        warehouse_cnt,
                        *c.threads.last().unwrap(),
                        None,
                        25,
                        25,
                        customer_cache_mb,
                        25,
                        stock_cache_mb,
                        25,
                    ),
                    c.clone(),
                );
                shumai::run(&mut payment_bench, &c, repeat)
            }
            CachePolicy::Tuple => {
                let mut payment_bench = TpccBench::<
                    WarehouseTuple,
                    DistrictTuple,
                    CustomerTuple,
                    ItemTuple,
                    StockTuple,
                    OrderTuple,
                    OrderLineTuple,
                >::new(
                    warehouse_cnt,
                    TpccWorkload::new(
                        warehouse_cnt,
                        *c.threads.last().unwrap(),
                        None,
                        25,
                        25,
                        customer_cache_mb,
                        25,
                        stock_cache_mb,
                        25,
                    ),
                    c.clone(),
                );
                shumai::run(&mut payment_bench, &c, repeat)
            }
            CachePolicy::Direct => {
                let mut payment_bench = TpccBench::<
                    WarehouseTuple,
                    DistrictTuple,
                    CustomerTuple,
                    ItemTuple,
                    StockTuple,
                    OrderTuple,
                    OrderLineTuple,
                >::new(
                    warehouse_cnt,
                    TpccWorkload::new(
                        warehouse_cnt,
                        *c.threads.last().unwrap(),
                        Some(0),
                        25,
                        25,
                        customer_cache_mb,
                        25,
                        stock_cache_mb,
                        25,
                    ),
                    c.clone(),
                );
                shumai::run(&mut payment_bench, &c, repeat)
            }
        };

        results.write_json().unwrap();
    }

    Ok(())
}
