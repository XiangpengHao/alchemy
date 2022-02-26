use std::time::SystemTime;

use alchemy::{
    cache_manager::{Rid, Schema},
    error::TransactionError,
    index::con_art_rust::{Key, UsizeKey},
};
use metric::{histogram, Histogram};
use rand::{thread_rng, Rng};

use crate::{
    mk_field, mk_query,
    tpcc::{
        schemas::GeneralSchema,
        tables::{
            CustomerTuple, DistrictTuple, ItemTuple, NewOrderTuple, OrderLineTuple, OrderTuple,
            StockTuple, WarehouseTuple,
        },
        workload::TpccWorkload,
        DISTRICT_SCALE, ITEM_SCALE,
    },
};
use bincode::Encode;

#[derive(Encode)]
pub struct DeliveryInput {
    w_id: u32,
    o_carrier_id: u32,
}

impl DeliveryInput {
    pub fn gen(warehouse_cnt: usize, task_id: usize, task_cnt: usize) -> Self {
        let mut rng = thread_rng();

        let w_per_thread = warehouse_cnt / task_cnt;
        let w_id = rng
            .gen_range((w_per_thread * task_id)..(w_per_thread * (task_id + 1)).min(warehouse_cnt));

        Self {
            w_id: w_id as u32,
            o_carrier_id: rng.gen_range(1..=10),
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
    pub async fn delivery(&self, input: &DeliveryInput) -> Result<(), TransactionError> {
        let guard = self.warehouse.pin();
        let art_guard = self.new_order.pin();

        let o_fields = mk_query!(OrderTuple, o_c_id, o_carrier_id);
        let ol_fields = mk_query!(OrderLineTuple, ol_delivery_d, ol_amount);
        let c_fields = mk_query!(CustomerTuple, c_balance, c_delivery_cnt);

        let mut rid_buffer = [Rid::from_u32(0); 20];
        let delivery_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;

        // Iterate over all district
        for d_id in 0..DISTRICT_SCALE {
            /*  Scan for the oldest new order ID. */
            let ol_key_low = NewOrderTuple::make_key(input.w_id as u64, d_id as u64, 0, 0);
            let ol_key_high = NewOrderTuple::make_key(
                input.w_id as u64,
                d_id as u64,
                u16::MAX as u64,
                u16::MAX as u64,
            );

            let mut out_buffer = [0; 1];
            let scanned = self.new_order.look_up_range(
                &UsizeKey::key_from(ol_key_low),
                &UsizeKey::key_from(ol_key_high),
                &mut out_buffer,
            );

            match scanned {
                Some(_cnt) => {
                    self.new_order
                        .remove(&UsizeKey::key_from(out_buffer[0]), &art_guard);
                }
                None => {
                    metric::counter!(metric::Counter::DeliveryNewOrderNotFound, 1);
                    return Ok(());
                }
            }
            let (o_id, c_id) = NewOrderTuple::o_c_id_from_key(out_buffer[0]);

            /* Read the oldest order */
            let order_key =
                OrderTuple::make_key(input.w_id as u64, d_id as u64, c_id as u64, o_id as u64);
            let o_lock = self.order.write_oid(&order_key, &art_guard).await?;
            let o_result = self.order.read(&o_lock, &o_fields);
            o_result.prefetch_all().await;
            let o_carrier_id: &mut u64 = unsafe {
                o_result.get_value_mut(&mk_field!(OrderTuple, o_carrier_id), self.order.schema())
            };
            *o_carrier_id = input.o_carrier_id as u64;

            let o_c_id: u64 =
                unsafe { *o_result.get_value(&mk_field!(OrderTuple, o_c_id), self.order.schema()) };

            assert_eq!(o_c_id, c_id as u64);

            /* Scan the matching order lines */
            let ol_low_key = OrderLineTuple::make_key(input.w_id as usize, d_id, o_id, 0);
            let ol_high_key =
                OrderLineTuple::make_key(input.w_id as usize, d_id, o_id, ITEM_SCALE as usize);
            let scanned = self
                .order_line
                .range(ol_low_key, ol_high_key, &mut rid_buffer)
                .expect("failed to scan the order lines");
            histogram!(Histogram::OrderLineScan, scanned as u64);

            /* Iterate over all order lines to get the sum of ol_amount */
            let mut ol_amount_sum = 0.0;
            for r in rid_buffer.iter().take(scanned) {
                let mut ol_lock = self.order_line.lock_rid_w(*r).await?;
                let ol_results = self
                    .order_line
                    .read_and_promote(&mut ol_lock, &ol_fields)
                    .await;
                ol_results.prefetch_all().await;
                let ol_delivery_d: &mut usize = unsafe {
                    ol_results.get_value_mut(
                        &mk_field!(OrderLineTuple, ol_delivery_d),
                        self.order_line.schema(),
                    )
                };
                *ol_delivery_d = delivery_time;

                let ol_amount: f64 = unsafe {
                    *ol_results.get_value(
                        &mk_field!(OrderLineTuple, ol_amount),
                        self.order_line.schema(),
                    )
                };
                ol_amount_sum += ol_amount;
            }

            /* Read the customer and update the fields */
            let c_key = CustomerTuple::make_key(input.w_id as u64, d_id as u64, o_c_id);
            let mut c_lock = self.customer.write_oid(&c_key, &guard).await?;
            let c_results = self.customer.read_and_promote(&mut c_lock, &c_fields).await;
            c_results.prefetch_all().await;
            let c_balance: &mut f64 = unsafe {
                c_results
                    .get_value_mut(&mk_field!(CustomerTuple, c_balance), self.customer.schema())
            };
            *c_balance += ol_amount_sum;

            let c_delivery_cnt: &mut u32 = unsafe {
                c_results.get_value_mut(
                    &mk_field!(CustomerTuple, c_delivery_cnt),
                    self.customer.schema(),
                )
            };
            *c_delivery_cnt += 1;
        }

        Ok(())
    }
}
