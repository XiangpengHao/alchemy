use alchemy::{
    cache_manager::{Rid, Schema},
    error::TransactionError,
};
use metric::{histogram, Histogram};
use rand::{thread_rng, Rng};

use crate::{
    mk_field, mk_query,
    tpcc::{
        schemas::GeneralSchema,
        tables::{
            CustomerTuple, DistrictTuple, ItemTuple, OrderLineTuple, OrderTuple, StockTuple,
            WarehouseTuple,
        },
        workload::TpccWorkload,
        CUSTOMER_SCALE, DISTRICT_SCALE,
    },
    utils::nu_rand,
};

pub struct OrderStatusInput {
    w_id: u32,
    d_id: u32,
    c_query: CustomerQuery,
}

enum CustomerQuery {
    Id(u32),
    LastName(u32),
}

impl OrderStatusInput {
    pub fn gen(warehouse_cnt: usize, task_id: usize, task_cnt: usize) -> Self {
        let mut rng = thread_rng();

        let w_per_thread = warehouse_cnt / task_cnt;
        let w_id = rng
            .gen_range((w_per_thread * task_id)..(w_per_thread * (task_id + 1)).min(warehouse_cnt));
        let d_id = rng.gen_range(0..DISTRICT_SCALE);
        let c_query = if rng.gen_range(0..100) < 60 {
            CustomerQuery::LastName(nu_rand(255, 0, 999) as u32)
        } else {
            CustomerQuery::Id(nu_rand(1023, 0, (CUSTOMER_SCALE - 1) as u64) as u32)
        };

        OrderStatusInput {
            w_id: w_id as u32,
            d_id: d_id as u32,
            c_query,
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
    pub async fn order_status(&self, input: &OrderStatusInput) -> Result<(), TransactionError> {
        let guard = self.warehouse.pin();

        /* Read customer */
        let mut c_lock = match input.c_query {
            CustomerQuery::Id(id) => {
                let c_key =
                    CustomerTuple::make_key(input.w_id as u64, input.d_id as u64, id as u64);
                self.customer.write_oid(&c_key, &guard).await?
            }
            CustomerQuery::LastName(seed) => {
                let low = CustomerTuple::idx_key_lastname(
                    input.w_id as u16,
                    input.d_id as u16,
                    seed as u16,
                    0,
                );
                let high = CustomerTuple::idx_key_lastname(
                    input.w_id as u16,
                    input.d_id as u16,
                    seed as u16,
                    CUSTOMER_SCALE as u16,
                );

                // we currently allow 8 rid to be scanned, this should be enough because we expect to scan only 3 items
                const RID_MAX_COUNT: usize = 8;
                let mut rid_buffer = [(0, 0); RID_MAX_COUNT];

                let cnt = self.c_last_idx.range(low, high, &mut rid_buffer, &guard);
                let middle_rid = if cnt == 0 {
                    histogram!(Histogram::PaymentCustomerScan, 0);
                    return Ok(());
                } else {
                    histogram!(Histogram::PaymentCustomerScan, cnt as u64);
                    Rid::from_u32(rid_buffer[cnt / 2].1 as u32)
                };

                self.customer
                    .lock_rid_w(middle_rid)
                    .await
                    .map_err(|e| match e {
                        TransactionError::IndexNotFound => TransactionError::UserAbort,
                        _ => e,
                    })?
            }
        };

        let c_fields = mk_query!(CustomerTuple, c_id, c_balance, c_first, c_middle, c_last);
        let c_results = self.customer.read_and_promote(&mut c_lock, &c_fields).await;
        c_results.prefetch_all().await;
        unsafe {
            for i in c_fields.iter() {
                let _v: u8 = *c_results.get_value(i, self.customer.schema());
            }
        }

        let c_id: usize = unsafe {
            *c_results.get_value(&mk_field!(CustomerTuple, c_id), self.customer.schema())
        };

        // Scan and get the most recent order made by this customer.
        let order_low = OrderTuple::make_key(input.w_id as u64, input.d_id as u64, c_id as u64, 0);
        let order_high = OrderTuple::make_key(
            input.w_id as u64,
            input.d_id as u64,
            c_id as u64,
            u16::MAX as u64,
        );
        let mut rid_buffer = [Rid::from_u32(0); 12];

        let scanned = self.order.range(order_low, order_high, &mut rid_buffer, &guard);
        let most_recent = match scanned {
            Some(cnt) => {
                histogram!(Histogram::OSOrderScan, cnt as u64);
                rid_buffer[cnt - 1]
            }
            None => {
                histogram!(Histogram::OSOrderScan, 0);
                return Ok(());
            }
        };

        // Read this order
        let o_lock = self.order.lock_rid_r(most_recent).await?;
        let o_fields = mk_query!(OrderTuple, o_id, o_entry_d, o_carrier_id);
        let o_results = self.order.read(&o_lock, &o_fields);
        o_results.prefetch_all().await;

        unsafe {
            for i in o_fields.iter() {
                let _v: u8 = *o_results.get_value(i, self.order.schema());
            }
        }
        let o_id: u64 =
            unsafe { *o_results.get_value(&mk_field!(OrderTuple, o_id), self.order.schema()) };

        // Scan the order line table
        let ol_low =
            OrderLineTuple::make_key(input.w_id as usize, input.d_id as usize, o_id as usize, 0);
        let ol_high = OrderLineTuple::make_key(
            input.w_id as usize,
            input.d_id as usize,
            o_id as usize,
            u16::MAX as usize,
        );

        let scanned = self.order_line.range(ol_low, ol_high, &mut rid_buffer, &guard);
        let cnt = match scanned {
            Some(cnt) => {
                histogram!(Histogram::OrderLineScan, cnt as u64);
                cnt
            }
            None => {
                histogram!(Histogram::OrderLineScan, 0);
                return Ok(());
            }
        };

        // Read the all the order lines from the table
        let ol_fields = mk_query!(
            OrderLineTuple,
            ol_i_id,
            ol_supply_w_id,
            ol_quantity,
            ol_amount,
            ol_delivery_d
        );
        for r in rid_buffer.iter().take(cnt) {
            let o_lock = self.order_line.lock_rid_r(*r).await?;
            let ol_results = self.order_line.read(&o_lock, &ol_fields);
            ol_results.prefetch_all().await;
            unsafe {
                for i in ol_fields.iter() {
                    let _v: u8 = *ol_results.get_value(i, self.order_line.schema());
                }
            }
        }
        Ok(())
    }
}
