use crate::tpcc::{
    schemas::GeneralSchema,
    tables::{
        CustomerTuple, DistrictTuple, ItemTuple, OrderLineTuple, OrderTuple, StockTuple,
        WarehouseTuple,
    },
    workload::TpccWorkload,
    DISTRICT_SCALE,
};
use crate::{mk_field, mk_query};
use bincode::Encode;
use metric::{histogram, Histogram};

use alchemy::{
    cache_manager::{Rid, Schema},
    error::TransactionError,
};
use rand::{thread_rng, Rng};

#[derive(Encode)]
pub struct StockLevelInput {
    w_id: u32,
    d_id: u32,
    stock_threshold: u32,
}

impl StockLevelInput {
    pub fn gen(warehouse_cnt: usize, task_id: usize, task_cnt: usize) -> Self {
        let mut rng = thread_rng();

        let w_per_thread = warehouse_cnt / task_cnt;
        let w_id = rng
            .gen_range((w_per_thread * task_id)..(w_per_thread * (task_id + 1)).min(warehouse_cnt));
        let d_id = rng.gen_range(0..DISTRICT_SCALE); // district = w * 10

        let threshold = rng.gen_range(10..=20);

        StockLevelInput {
            w_id: w_id as u32,
            d_id: d_id as u32,
            stock_threshold: threshold,
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
    pub async fn stock_level(&self, input: &StockLevelInput) -> Result<(), TransactionError> {
        let guard = self.warehouse.pin();

        /* Read the district with matching keys */
        let d_key = DistrictTuple::make_key(input.w_id as u64, input.d_id as u64);
        let d_lock = self.district.read_oid_sync(&d_key, &guard)?;
        let d_fields = mk_query!(DistrictTuple, d_id, d_next_o_id);
        let d_result = self.district.read(&d_lock, &d_fields);
        let d_next_o_id: u64 = unsafe {
            *d_result.get_value(
                &mk_field!(DistrictTuple, d_next_o_id),
                self.district.schema(),
            )
        };

        /* Scan the order line table for the most recent 20 records */
        assert!(d_next_o_id >= 20);
        let ol_low = OrderLineTuple::make_key(
            input.w_id as usize,
            input.d_id as usize,
            std::cmp::min(0, d_next_o_id as usize - 20),
            0,
        );
        let ol_high = OrderLineTuple::make_key(
            input.w_id as usize,
            input.d_id as usize,
            d_next_o_id as usize,
            u16::MAX as usize,
        );

        let mut ol_rids = vec![Rid::from_u32(0); 128];
        let scanned = self
            .order_line
            .range(ol_low, ol_high, &mut ol_rids)
            .unwrap();
        histogram!(Histogram::OrderLineScan, scanned as u64);

        let mut result_cnt = 0;
        /* Read the order line table for the ol_i_id */
        let ol_fields = mk_query!(OrderLineTuple, ol_d_id, ol_o_id, ol_i_id);
        let stock_fields = mk_query!(StockTuple, s_w_id, s_i_id);
        for r in ol_rids.iter().take(scanned) {
            let ol_lock = self.order_line.lock_rid_r(*r).await?;
            let ol_results = self.order_line.read(&ol_lock, &ol_fields);
            ol_results.prefetch_all().await;
            let ol_i_id: u64 = unsafe {
                *ol_results.get_value(
                    &mk_field!(OrderLineTuple, ol_i_id),
                    self.order_line.schema(),
                )
            };

            /* For every ol_i_id, read its stock level */
            let s_key = StockTuple::make_key(input.w_id as u64, ol_i_id as u64);
            let s_lock = self.stock.read_oid(&s_key, &guard).await?;
            let s_results = self.stock.read(&s_lock, &stock_fields);
            s_results.prefetch_all().await;
            let s_quantity: u64 = unsafe {
                *s_results.get_value(&mk_field!(StockTuple, s_quantity), self.stock.schema())
            };
            if s_quantity < input.stock_threshold as u64 {
                result_cnt += 1;
            }
        }

        histogram!(Histogram::StockLevelQuantity, result_cnt as u64);
        Ok(())
    }
}
