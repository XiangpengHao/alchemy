use std::time::SystemTime;

use alchemy::{
    attribute_cache::{
        clock_cache::oid::{OidReadGuard, OidWriteGuard},
        Schema,
    },
    block_on,
    error::TransactionError,
};
use rand::{prelude::SmallRng, thread_rng, Rng, SeedableRng};

use crate::tpcc::{
    schemas::GeneralSchema,
    tables::{
        CustomerTuple, DistrictTuple, ItemTuple, NewOrderTuple, OrderLineTuple, OrderTuple,
        StockTuple, WarehouseTuple,
    },
    workload::TpccWorkload,
    CARRIER_NULL, CUSTOMER_SCALE, DISTRICT_SCALE, ITEM_SCALE,
};
use crate::{
    mk_field, mk_query,
    utils::{nu_rand, FixedString},
};
use bincode::Encode;

#[derive(Default, Encode)]
struct OrderItem {
    ol_i_id: u32,
    ol_supply_w_id: u16,
    ol_quantity: u16,
}

#[derive(Encode)]
pub struct NewOrderInput {
    w_id: u32,
    d_id: u32,
    c_id: u32,
    ol_cnt: u32,
    orders: [OrderItem; 15],
    o_entry_d: u64,
    all_local: bool,
}

impl NewOrderInput {
    pub fn gen(warehouse_cnt: usize, task_id: usize, task_cnt: usize) -> Self {
        let rng = thread_rng();
        let mut rng = SmallRng::from_rng(rng).unwrap();

        let w_per_thread = warehouse_cnt / task_cnt;
        let w_id = rng
            .gen_range((w_per_thread * task_id)..(w_per_thread * (task_id + 1)).min(warehouse_cnt));
        let d_id = rng.gen_range(0..DISTRICT_SCALE); // district = w * 10
        let c_id = nu_rand(1023, 0, (CUSTOMER_SCALE - 1) as u64);
        let ol_cnt = rng.gen_range(5..=15);
        let rbk = rng.gen_bool(0.01);

        let mut orders: [OrderItem; 15] = Default::default();
        let mut all_local = true;

        let i_id_start = nu_rand(8191, 0, (ITEM_SCALE - 1) as u64);
        for (i, order) in orders.iter_mut().enumerate().take(ol_cnt) {
            let ol_i_id = if i == (ol_cnt - 1) && rbk {
                (ITEM_SCALE + 1) as u64 // unused value to trigger rollback
            } else {
                i_id_start + i as u64
            };
            let ol_supply_w_id = if rng.gen_bool(0.01) {
                all_local = false;
                rng.gen_range(0..warehouse_cnt)
            } else {
                w_id
            };
            let ol_quantity = rng.gen_range(1..=10);
            *order = OrderItem {
                ol_i_id: ol_i_id as u32,
                ol_supply_w_id: ol_supply_w_id as u16,
                ol_quantity,
            };
        }

        let o_entry_d = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            w_id: w_id as u32,
            d_id: d_id as u32,
            o_entry_d,
            c_id: c_id as u32,
            ol_cnt: ol_cnt as u32,
            orders,
            all_local,
        }
    }
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
    /// New order:
    /// We always acquire the exclusive lock on the warehouse first, so that all other access the our current warehouse can abort early
    /// When the x-lock is granted, we can go ahead to read any data on the warehouse without worry about the concurrency control
    /// It's a implementation detail of tpcc new order, not anything in the database core.
    pub async fn new_order(&self, input: &NewOrderInput) -> Result<(), TransactionError> {
        let guard = self.warehouse.pin();
        let art_guard = self.order.pin();

        /*================================================= */
        /* Phase 1: acquire locks                           */
        /*================================================= */

        /* read warehouse */
        let w_key = input.w_id as usize;
        let mut w_lock = self.warehouse.write_oid_sync(&w_key, &guard)?;

        let w_query = mk_query!(WarehouseTuple, w_tax);
        let w_results = block_on(self.warehouse.read_and_promote(&mut w_lock, &w_query));

        /* read district */
        let d_key = DistrictTuple::make_key(input.w_id as u64, input.d_id as u64);
        let mut d_lock = self.district.write_oid_sync(&d_key, &guard)?;
        let d_query = mk_query!(DistrictTuple, d_tax, d_next_o_id);
        let d_results = block_on(self.district.read_and_promote(&mut d_lock, &d_query));

        /* read customer */
        let c_key =
            CustomerTuple::make_key(input.w_id as u64, input.d_id as u64, input.c_id as u64);
        let mut c_lock = self.customer.write_oid(&c_key, &guard).await?;
        let c_query = mk_query!(CustomerTuple, c_discount, c_last, c_credit);
        let c_results = self.customer.read_and_promote(&mut c_lock, &c_query).await;

        /* Before we do any harmful to our database, we first examine the input and acquire all the locks */
        let mut item_stock_locks: [Option<(OidReadGuard, OidWriteGuard)>; 15] = [
            None, None, None, None, None, None, None, None, None, None, None, None, None, None,
            None,
        ];
        for (i, lock) in item_stock_locks
            .iter_mut()
            .enumerate()
            .take(input.ol_cnt as usize)
        {
            let cur_order = &input.orders[i];
            let i_key = ItemTuple::make_key(cur_order.ol_i_id as u64);

            // if the i_key is not valid, we will return user-abort and release all the locks.
            let i_lock = self
                .item
                .read_oid_sync(&i_key, &guard)
                .map_err(|e| match e {
                    TransactionError::IndexNotFound => TransactionError::UserAbort,
                    _ => e,
                })?;

            let s_key =
                StockTuple::make_key(cur_order.ol_supply_w_id as u64, cur_order.ol_i_id as u64);

            let s_lock = self.stock.write_oid(&s_key, &guard).await?;
            lock.replace((i_lock, s_lock));
        }

        /*================================================= */
        /* Phase 2: Modify the system and release the locks */
        /*================================================= */

        unsafe {
            /* Touch all the fields to make sure we are not cheating */
            for i in w_query.iter() {
                let _: u8 = *w_results.get_value(i, self.warehouse.schema());
            }
            for i in d_query.iter() {
                let _: u8 = *d_results.get_value(i, self.district.schema());
            }

            c_results.prefetch_all().await;
            for i in c_query.iter() {
                let _: u8 = *c_results.get_value(i, self.customer.schema());
            }
        }

        let d_next_o_id: u64 = unsafe {
            let field = mk_field!(DistrictTuple, d_next_o_id);
            let o_id = d_results.get_value_mut(&field, self.district.schema());
            let old_o_id = *o_id;
            *o_id += 1;
            old_o_id
        };

        let no_key = NewOrderTuple::make_key(
            input.w_id as u64,
            input.d_id as u64,
            d_next_o_id,
            input.c_id as u64,
        );
        self.new_order.insert(no_key, no_key, &art_guard);

        let order = OrderTuple {
            o_id: d_next_o_id,
            o_c_id: input.c_id as u64,
            o_d_id: input.d_id as u64,
            o_carrier_id: CARRIER_NULL as u64,
            o_entry_d: input.o_entry_d,
            o_w_id: input.w_id as u64,
            o_ol_cnt: input.ol_cnt as u64,
            o_all_local: input.all_local as u64,
        };
        let _ = self.order.insert(order, &art_guard);

        // Insert order lines
        for i in 0..input.ol_cnt {
            let cur_order = &input.orders[i as usize];
            let (i_lock, mut s_lock) = item_stock_locks[i as usize].take().unwrap();

            /* read item */
            let i_query = mk_query!(ItemTuple, i_price, i_name, i_data);
            let i_results = self.item.read(&i_lock, &i_query);

            let i_price = unsafe {
                i_results.get_value::<f64, _>(&mk_field!(ItemTuple, i_price), self.item.schema())
            };

            unsafe {
                for i in i_query.iter() {
                    let _: u8 = *i_results.get_value(i, self.item.schema());
                }
            }

            /* read stock */
            // this is equivalent to having ten different schemas and each with one district cached in the schema
            let s_query = mk_query!(
                StockTuple,
                s_dist10,
                s_quantity,
                s_ytd,
                s_order_cnt,
                s_remote_cnt,
                s_data
            );
            let s_results = self.stock.read_and_promote(&mut s_lock, &s_query).await;
            s_results.prefetch_all().await;

            let s_dist_field = mk_field!(StockTuple, s_dist10);
            let s_dist_info = unsafe {
                s_results.get_value::<FixedString<24>, _>(&s_dist_field, self.stock.schema())
            };

            unsafe {
                let field = mk_field!(StockTuple, s_quantity);
                let s_quantity: &mut u64 = s_results.get_value_mut(&field, self.stock.schema());

                if (*s_quantity - cur_order.ol_quantity as u64) as isize >= 10 {
                    *s_quantity -= cur_order.ol_quantity as u64;
                } else {
                    *s_quantity =
                        ((*s_quantity - cur_order.ol_quantity as u64) as isize + 91) as u64;
                }
            };

            unsafe {
                let field = mk_field!(StockTuple, s_ytd);
                let s_ytd: &mut u64 = s_results.get_value_mut(&field, self.stock.schema());
                *s_ytd += cur_order.ol_quantity as u64;
            };

            unsafe {
                let field = mk_field!(StockTuple, s_order_cnt);
                let s_order_cnt: &mut u64 = s_results.get_value_mut(&field, self.stock.schema());
                *s_order_cnt += 1;
            };

            unsafe {
                if cur_order.ol_supply_w_id as u32 != input.w_id {
                    let field = mk_field!(StockTuple, s_remote_cnt);
                    let s_remote_cnt: &mut u64 =
                        s_results.get_value_mut(&field, self.stock.schema());
                    *s_remote_cnt += 1;
                }
            }

            let ol_amount = cur_order.ol_quantity as f64 * *i_price;

            let order_line = OrderLineTuple {
                ol_o_id: d_next_o_id as u64,
                ol_d_id: input.d_id as u64,
                ol_w_id: input.w_id as u64,
                ol_i_id: cur_order.ol_i_id as u64,
                ol_supply_w_id: cur_order.ol_supply_w_id as u64,
                ol_number: i as u64,
                ol_quantity: cur_order.ol_quantity as u64,
                ol_delivery_d: CARRIER_NULL as u64,
                ol_amount,
                ol_dist_info: s_dist_info.clone(),
            };

            let _ = self.order_line.insert(order_line, &art_guard);
        }
        Ok(())
    }
}
