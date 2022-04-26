use std::{str::FromStr, time::SystemTime};

use alchemy::{
    attribute_cache::Schema,
    index::{congee, Art, DbIndex},
    table::DbTable,
};

use metric::CtxCounter;
use rand::{prelude::SliceRandom, thread_rng, Rng};

use crate::tpcc::{
    schemas::GeneralSchema,
    tables::{
        CustomerTuple, DistrictTuple, ItemTuple, OrderLineTuple, OrderTuple, StockTuple,
        WarehouseTuple,
    },
    CUSTOMER_SCALE, DISTRICT_SCALE, ITEM_SCALE,
};
use crate::utils::{align_up, last_name_gen, nu_rand, zip_gen, FixedString};

use super::tables::NewOrderTuple;

type TpccTable<F> = DbTable<Art, GeneralSchema<F>>;

const MB: usize = 1024 * 1024;

// a scale to estimate the order size
fn scale_factor(thread_cnt: usize) -> f64 {
    1.0 - (1.0 / 50.0) * thread_cnt as f64
}

pub struct TpccWorkload<W, D, C, I, S, O, OL>
where
    GeneralSchema<W>: Schema<Tuple = WarehouseTuple>,
    GeneralSchema<D>: Schema<Tuple = DistrictTuple>,
    GeneralSchema<C>: Schema<Tuple = CustomerTuple>,
    GeneralSchema<I>: Schema<Tuple = ItemTuple>,
    GeneralSchema<S>: Schema<Tuple = StockTuple>,
    GeneralSchema<O>: Schema<Tuple = OrderTuple>,
    GeneralSchema<OL>: Schema<Tuple = OrderLineTuple>,
{
    pub(crate) warehouse_cnt: u32,
    pub(crate) warehouse: TpccTable<W>,
    pub(crate) district: TpccTable<D>,
    pub(crate) customer: TpccTable<C>,
    pub(crate) c_last_idx: Art,
    pub(crate) item: TpccTable<I>,
    pub(crate) stock: TpccTable<S>,
    pub(crate) order: TpccTable<O>,
    pub(crate) order_line: TpccTable<OL>,
    pub(crate) new_order: congee::ArtRaw<usize, usize>, // new order doesn't need a table, the index contains everything
}

impl<W, D, C, I, S, O, OL> TpccWorkload<W, D, C, I, S, O, OL>
where
    GeneralSchema<W>: Schema<Tuple = WarehouseTuple>,
    GeneralSchema<D>: Schema<Tuple = DistrictTuple>,
    GeneralSchema<C>: Schema<Tuple = CustomerTuple>,
    GeneralSchema<I>: Schema<Tuple = ItemTuple>,
    GeneralSchema<S>: Schema<Tuple = StockTuple>,
    GeneralSchema<O>: Schema<Tuple = OrderTuple>,
    GeneralSchema<OL>: Schema<Tuple = OrderLineTuple>,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        warehouse_cnt: usize,
        thread_cnt: usize,
        probe_len: Option<usize>,
        w_cache_size_mb: usize,
        d_cache_size_mb: usize,
        c_cache_size_mb: usize,
        i_cache_size_mb: usize,
        s_cache_size_mb: usize,
        o_cache_size_mb: usize,
    ) -> Self {
        let w_size = (warehouse_cnt as usize) * std::mem::size_of::<WarehouseTuple>();
        let w_size = align_up(w_size, MB);

        let warehouse = TpccTable::builder()
            .cache(w_cache_size_mb * MB)
            .storage(w_size)
            .schema(GeneralSchema::new())
            .probe_rng(0.5)
            .probe_len(probe_len)
            .build();

        let d_size =
            (warehouse_cnt * super::DISTRICT_SCALE) as usize * std::mem::size_of::<DistrictTuple>();
        let d_size = align_up(d_size, MB);
        let district = TpccTable::builder()
            .cache(d_cache_size_mb * MB)
            .storage(d_size)
            .schema(GeneralSchema::new())
            .probe_rng(0.5)
            .probe_len(probe_len)
            .build();

        let c_size = (warehouse_cnt * DISTRICT_SCALE * super::CUSTOMER_SCALE) as usize
            * std::mem::size_of::<CustomerTuple>();
        let c_size = align_up(c_size, MB);
        let customer = TpccTable::builder()
            .cache(c_cache_size_mb * MB)
            .storage(c_size)
            .schema(GeneralSchema::new())
            .metric_context(CtxCounter::Customer)
            .probe_len(probe_len)
            .probe_rng(0.5)
            .build();

        let i_size = ITEM_SCALE as usize * std::mem::size_of::<ItemTuple>();
        let i_size = align_up(i_size, MB);
        let item = TpccTable::builder()
            .cache(i_cache_size_mb * MB)
            .storage(i_size)
            .schema(GeneralSchema::new())
            .probe_len(probe_len)
            .probe_rng(0.9)
            .build();

        let s_size = (warehouse_cnt * ITEM_SCALE) as usize * std::mem::size_of::<StockTuple>();
        let s_size = align_up(s_size, MB);
        let stock = TpccTable::builder()
            .cache(s_cache_size_mb * MB)
            .storage(s_size)
            .schema(GeneralSchema::new())
            .metric_context(CtxCounter::Stock)
            .probe_len(probe_len)
            .probe_rng(0.5)
            .build();

        let initial_cnt = warehouse_cnt * DISTRICT_SCALE * CUSTOMER_SCALE;
        let order_per_thread = 1_100_000;
        let factor = scale_factor(thread_cnt);
        let o_cnt = ((order_per_thread * thread_cnt) as f64 * factor + initial_cnt as f64) as usize;
        let o_size = o_cnt * std::mem::size_of::<OrderTuple>();
        let o_size = align_up(o_size, MB);
        let order = TpccTable::builder()
            .cache(o_cache_size_mb * MB)
            .storage(o_size)
            .schema(GeneralSchema::new())
            .metric_context(CtxCounter::Order)
            .probe_len(Some(0))
            .probe_rng(0.0)
            .build();

        // Caching order line is not a good idea:
        //     Order lines are around 10x of an order, they are mostly sequential
        //     Order insertion is too fast such that it floods the cache
        let ol_cnt = o_cnt * 12;
        let ol_size = ol_cnt * std::mem::size_of::<OrderLineTuple>();
        let ol_size = align_up(ol_size, MB);
        let order_line = TpccTable::builder()
            .cache(10 * MB)
            .storage(ol_size)
            .schema(GeneralSchema::new())
            .probe_len(Some(0))
            .probe_rng(0.0)
            .build();

        Self {
            warehouse_cnt: warehouse_cnt as u32,
            warehouse,
            district,
            customer,
            item,
            stock,
            c_last_idx: Art::new(),
            order_line,
            new_order: congee::ArtRaw::new(),
            order,
        }
    }

    pub fn thread_init(&self) {
        self.warehouse.thread_init();
        self.district.thread_init();
        self.customer.thread_init();
        self.item.thread_init();
        self.stock.thread_init();
        self.order.thread_init();
        self.order_line.thread_init();
    }

    pub fn load(&self) {
        // use 32 threads to load
        let warehouse_per_thread = self.warehouse_cnt / 32;
        assert!(
            self.warehouse_cnt % 32 == 0,
            "warehouse count must be a multiply of 32"
        );

        crossbeam_utils::thread::scope(|s| {
            let mut loading_handlers = Vec::new();

            for tid in 0..32 {
                let handle = s.spawn(move |_| {
                    self.thread_init();
                    let w_slices = warehouse_per_thread * tid..warehouse_per_thread * (tid + 1);
                    let art_guard = self.c_last_idx.pin();
                    let mut rng = thread_rng();

                    /* Load warehouse */
                    for w_id in w_slices {
                        let tuple = WarehouseTuple {
                            w_id: w_id as u64,
                            w_name: FixedString::rand_gen(),
                            w_street1: FixedString::rand_gen(),
                            w_street2: FixedString::rand_gen(),
                            w_city: FixedString::rand_gen(),
                            w_state: FixedString::rand_gen(),
                            w_tax: rng.gen_range(0.0..=0.2),
                            w_ytd: 300_000.00,
                            w_zip: zip_gen(),
                        };
                        let _lock = self.warehouse.insert(tuple, &art_guard);

                        /* Load district */
                        for d_id in 0..super::DISTRICT_SCALE {
                            let tuple = DistrictTuple {
                                d_id: d_id as u64,
                                d_w_id: w_id as u64,
                                d_name: FixedString::rand_gen(),
                                d_street1: FixedString::rand_gen(),
                                d_street2: FixedString::rand_gen(),
                                d_city: FixedString::rand_gen(),
                                d_state: FixedString::rand_gen(),
                                d_zip: zip_gen(),
                                d_tax: rng.gen_range(0.0..=0.2),
                                d_next_o_id: 3001,
                                d_ytd: 30_000.0,
                            };
                            let _lock = self.district.insert(tuple, &art_guard);

                            /* load customer */
                            let current_time = SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .unwrap()
                                .as_secs();
                            for c_id in 0..CUSTOMER_SCALE {
                                self.load_one_customer(
                                    c_id as u64,
                                    d_id as u64,
                                    w_id as u64,
                                    current_time,
                                    &art_guard,
                                );
                            }

                            self.load_order(w_id as u64, d_id as u64, current_time);
                        }

                        self.load_stock(w_id as u64);
                    }
                });
                loading_handlers.push(handle);
            }

            for h in loading_handlers.into_iter() {
                h.join().unwrap();
            }
        })
        .unwrap();

        self.load_item();
    }

    fn load_item(&self) {
        let mut rng = thread_rng();
        let guard = self.warehouse.pin();
        for i_id in 0..ITEM_SCALE {
            let i_im_id = rng.gen_range(1..=10_000);
            let i_name = FixedString::rand_gen();
            let i_price = rng.gen_range(1.0..=100.0);
            let i_data = if rng.gen_bool(0.1) {
                let mut origin = FixedString::rand_gen();
                origin.copy_from("ORIGINAL", rng.gen_range(0..42));
                origin
            } else {
                FixedString::rand_gen()
            };
            let _lock = self.item.insert(
                ItemTuple {
                    i_id: i_id as u64,
                    i_im_id,
                    i_name,
                    i_price,
                    i_data,
                },
                &guard,
            );
        }
    }

    fn load_one_customer(
        &self,
        c_id: u64,
        d_id: u64,
        w_id: u64,
        customer_since: u64,
        art_guard: &congee::epoch::Guard,
    ) {
        let mut rng = thread_rng();

        // According to page 64 in TPCC spec
        let seed = if c_id < 1000 {
            rng.gen_range(0..=999)
        } else {
            nu_rand(255, 0, 999)
        };

        let credit = if rng.gen_bool(0.1) {
            FixedString::from_str("BC").unwrap()
        } else {
            FixedString::from_str("GC").unwrap()
        };

        let tuple = CustomerTuple {
            c_id,
            c_d_id: d_id,
            c_w_id: w_id,
            c_last: last_name_gen(seed as u32),
            c_middle: FixedString::from_str("OE").unwrap(),
            c_first: FixedString::rand_gen(),
            c_street1: FixedString::rand_gen(),
            c_street2: FixedString::rand_gen(),
            c_city: FixedString::rand_gen(),
            c_state: FixedString::rand_gen(),
            c_zip: zip_gen(),
            c_phone: FixedString::rand_gen(),
            c_since: customer_since,
            c_credit: credit,
            c_credit_lim: 50_000.0,
            c_discount: rng.gen_range(0.0..=0.5),
            c_balance: -10.0,
            c_ytd_payment: 10.0,
            c_payment_cnt: 1,
            c_delivery_cnt: 0,
            c_data: FixedString::rand_gen(),
        };

        let last_key =
            CustomerTuple::idx_key_lastname(w_id as u16, d_id as u16, seed as u16, c_id as u16);

        let c_lock = self.customer.insert(tuple, art_guard);

        let c_rid = c_lock.0;
        self.c_last_idx.insert(last_key, c_rid, art_guard);
    }

    // Used to reset the index and table of orders, so that they are the same after each iteration
    pub fn reset_order(&mut self, _thread_cnt: usize) {
        self.order_line.reset();
        self.order.reset();
        self.new_order = congee::ArtRaw::new();

        // use 32 threads to load
        let warehouse_per_thread = self.warehouse_cnt / 32;
        assert!(
            self.warehouse_cnt % 32 == 0,
            "warehouse count must be a multiply of 32"
        );

        crossbeam_utils::thread::scope(|s| {
            let mut loading_handlers = Vec::new();

            for tid in 0..32 {
                let fresh_self = &*self;
                let handle = s.spawn(move |_| {
                    let w_slices = warehouse_per_thread * tid..warehouse_per_thread * (tid + 1);
                    for w_id in w_slices {
                        for d_id in 0..super::DISTRICT_SCALE {
                            let current_time = SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .unwrap()
                                .as_secs();

                            fresh_self.load_order(w_id as u64, d_id as u64, current_time);
                        }
                    }
                });
                loading_handlers.push(handle);
            }

            for h in loading_handlers.into_iter() {
                h.join().unwrap();
            }
        })
        .unwrap();
    }

    // Load order, order line and new order
    fn load_order(&self, w_id: u64, d_id: u64, current_time: u64) {
        let mut rng = thread_rng();
        let mut o_c_ids = Vec::with_capacity(CUSTOMER_SCALE);
        for i in 0..CUSTOMER_SCALE {
            o_c_ids.push(i);
        }
        o_c_ids.shuffle(&mut rng);
        let order_guard = self.order.pin();

        for (o, o_c_id) in o_c_ids.iter().enumerate() {
            let o_id = o;
            let o_w_id = w_id;
            let o_d_id = d_id;
            let o_entry_d = current_time;
            let o_carrier_id = if o_id < 2_101 {
                rng.gen_range(1..=10)
            } else {
                0
            };
            let o_ol_cnt = rng.gen_range(5..=15);
            let o_all_local = 1;
            let order_tuple = OrderTuple {
                o_id: o_id as u64,
                o_c_id: *o_c_id as u64,
                o_w_id,
                o_d_id,
                o_ol_cnt,
                o_entry_d,
                o_carrier_id,
                o_all_local,
            };
            let _o_lock = self.order.insert(order_tuple, &order_guard);

            let i_start = rng.gen_range(0..(ITEM_SCALE as u64 - o_ol_cnt));
            for ol in 0..o_ol_cnt {
                let ol_o_id = o_id;
                let ol_d_id = d_id;
                let ol_w_id = w_id;
                let ol_number = o_ol_cnt;
                let ol_i_id = i_start + ol;
                let ol_supply_w_id = w_id;
                let ol_delivery_d = if ol_o_id < 2101 { o_entry_d } else { 0 };
                let ol_quantity = 5;
                let ol_amount = if ol_o_id < 2101 {
                    0.0
                } else {
                    rng.gen_range(0.01..9_999.99)
                };
                let ol_dist_info = FixedString::rand_gen();
                let ol_tuple = OrderLineTuple {
                    ol_o_id: ol_o_id as u64,
                    ol_d_id,
                    ol_w_id,
                    ol_number,
                    ol_i_id,
                    ol_supply_w_id,
                    ol_delivery_d,
                    ol_quantity,
                    ol_amount,
                    ol_dist_info,
                };

                let _ol_lock = self.order_line.insert(ol_tuple, &order_guard);
            }

            if o >= 2101 {
                // insert to new order
                let key = NewOrderTuple::make_key(w_id, d_id, o as u64, *o_c_id as u64);
                self.new_order.insert(key, key, &order_guard);
            }
        }
    }

    fn load_stock(&self, w_id: u64) {
        let mut rng = thread_rng();
        let guard = self.stock.pin();
        for s_i_id in 0..ITEM_SCALE {
            let s_data = if rng.gen_bool(0.1) {
                let mut origin = FixedString::rand_gen();
                origin.copy_from("ORIGINAL", rng.gen_range(0..42));
                origin
            } else {
                FixedString::rand_gen()
            };
            let stock = StockTuple {
                s_i_id: s_i_id as u64,
                s_w_id: w_id,
                s_quantity: rng.gen_range(10..=100),
                s_dist01: FixedString::rand_gen(),
                s_dist02: FixedString::rand_gen(),
                s_dist03: FixedString::rand_gen(),
                s_dist04: FixedString::rand_gen(),
                s_dist05: FixedString::rand_gen(),
                s_dist06: FixedString::rand_gen(),
                s_dist07: FixedString::rand_gen(),
                s_dist08: FixedString::rand_gen(),
                s_dist09: FixedString::rand_gen(),
                s_dist10: FixedString::rand_gen(),
                s_ytd: 0,
                s_order_cnt: 0,
                s_remote_cnt: 0,
                s_data,
            };
            let _s_lock = self.stock.insert(stock, &guard);
        }
    }
}
