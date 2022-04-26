use crate::tpcc::{
    schemas::GeneralSchema,
    tables::{
        CustomerTuple, DistrictTuple, HistoryTuple, ItemTuple, OrderLineTuple, OrderTuple,
        StockTuple, WarehouseTuple, C_DATA_SIZE,
    },
    workload::TpccWorkload,
    CUSTOMER_SCALE, DISTRICT_SCALE,
};
use crate::{
    mk_field, mk_query,
    utils::{nu_rand, FixedString},
};
use bincode::Encode;
use metric::{histogram, Histogram};

use alchemy::{
    attribute_cache::{Rid, Schema},
    block_on,
    error::TransactionError,
};
use rand::{prelude::SmallRng, thread_rng, Rng, SeedableRng};
use std::time::SystemTime;

#[derive(Encode)]
pub struct PaymentInput {
    w_id: u32,
    d_id: u32,
    c_w_id: u32,
    c_d_id: u32,
    c_query: CustomerQuery,
    h_amount: f64,
    h_date: u64,
}

#[derive(Encode)]
enum CustomerQuery {
    Id(u32),
    LastName(u32),
}

impl PaymentInput {
    pub fn gen(warehouse_cnt: usize, task_id: usize, task_cnt: usize) -> Self {
        let rng = thread_rng();
        let mut rng = SmallRng::from_rng(rng).unwrap();

        let w_per_thread = warehouse_cnt / task_cnt;
        let w_id = rng
            .gen_range((w_per_thread * task_id)..(w_per_thread * (task_id + 1)).min(warehouse_cnt));
        let d_id = rng.gen_range(0..DISTRICT_SCALE); // district = w * 10
        let c_query = if rng.gen_range(0..100) < 60 {
            CustomerQuery::LastName(nu_rand(255, 0, 999) as u32)
        } else {
            // customer = 3k
            CustomerQuery::Id(nu_rand(1023, 0, (CUSTOMER_SCALE - 1) as u64) as u32)
        };

        let h_amount = rng.gen_range(1.0..5000.0);
        let h_date = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let (c_w_id, c_d_id) = if rng.gen_range(0..100) < 85 {
            (w_id, d_id) // home warehouse and district
        } else {
            // remote warehouse and district
            let c_w_id = rng.gen_range(0..warehouse_cnt);
            let c_d_id = rng.gen_range(0..DISTRICT_SCALE);
            (c_w_id, c_d_id)
        };

        Self {
            w_id: w_id as u32,
            d_id: d_id as u32,
            c_w_id: c_w_id as u32,
            c_d_id: c_d_id as u32,
            c_query,
            h_amount,
            h_date,
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
    pub async fn payment(&self, input: &PaymentInput) -> Result<(), TransactionError> {
        let guard = self.warehouse.pin();

        /* read warehouse */
        let w_key = input.w_id as usize;
        let mut w_lock = self.warehouse.write_oid_sync(&w_key, &guard)?;
        let w_fields = mk_query!(
            WarehouseTuple,
            w_name,
            w_street1,
            w_street2,
            w_city,
            w_state,
            w_zip,
            w_ytd
        );

        let w_result = block_on(self.warehouse.read_and_promote(&mut w_lock, &w_fields));
        let w_ytd: &mut f64 = unsafe {
            w_result.get_value_mut(&mk_field!(WarehouseTuple, w_ytd), self.warehouse.schema())
        };

        let w_name = unsafe {
            w_result.get_value::<FixedString<10>, GeneralSchema<W>>(
                &mk_field!(WarehouseTuple, w_name),
                self.warehouse.schema(),
            )
        };
        /* touch every fields so that they are actually loaded from memory */
        unsafe {
            for i in w_fields.iter() {
                let _: u8 = *w_result.get_value(i, self.warehouse.schema());
            }
        }
        *w_ytd += input.h_amount;

        /* read district */
        let d_key = DistrictTuple::make_key(input.w_id as u64, input.d_id as u64);
        let mut d_lock = self.district.write_oid_sync(&d_key, &guard)?;
        let d_fields = mk_query!(
            DistrictTuple,
            d_name,
            d_street1,
            d_street2,
            d_city,
            d_state,
            d_zip,
            d_ytd
        );
        let d_result = block_on(self.district.read_and_promote(&mut d_lock, &d_fields));
        let d_ytd: &mut f64 = unsafe {
            d_result.get_value_mut(&mk_field!(DistrictTuple, d_ytd), self.district.schema())
        };
        *d_ytd += input.h_amount;

        let d_name = unsafe {
            d_result.get_value::<FixedString<10>, _>(
                &mk_field!(DistrictTuple, d_name),
                self.district.schema(),
            )
        };

        unsafe {
            for i in d_fields.iter() {
                let _: u8 = *d_result.get_value(i, self.district.schema());
            }
        }

        /* read customers */
        let mut c_lock = match input.c_query {
            CustomerQuery::Id(id) => {
                let c_key =
                    CustomerTuple::make_key(input.c_w_id as u64, input.c_d_id as u64, id as u64);
                self.customer.write_oid(&c_key, &guard).await?
            }
            CustomerQuery::LastName(seed) => {
                let low = CustomerTuple::idx_key_lastname(
                    input.c_w_id as u16,
                    input.c_d_id as u16,
                    seed as u16,
                    0,
                );

                let high = CustomerTuple::idx_key_lastname(
                    input.c_w_id as u16,
                    input.c_d_id as u16,
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
        let c_fields = mk_query!(
            CustomerTuple,
            c_first,
            c_middle,
            c_last,
            c_street1,
            c_street2,
            c_city,
            c_state,
            c_zip,
            c_phone,
            c_since,
            c_credit,
            c_discount,
            c_balance,
            c_ytd_payment,
            c_payment_cnt
        );

        let c_results = self.customer.read_and_promote(&mut c_lock, &c_fields).await;
        c_results.prefetch_all().await;
        unsafe {
            for i in c_fields.iter() {
                let _v: u8 = *c_results.get_value(i, self.customer.schema());
            }
        }
        let c_balance: &mut f64 = unsafe {
            c_results.get_value_mut(&mk_field!(CustomerTuple, c_balance), self.customer.schema())
        };
        *c_balance -= input.h_amount;

        let c_ytd_payment: &mut f64 = unsafe {
            c_results.get_value_mut(
                &mk_field!(CustomerTuple, c_ytd_payment),
                self.customer.schema(),
            )
        };
        *c_ytd_payment += input.h_amount;

        let c_payment_cnt: &mut u32 = unsafe {
            c_results.get_value_mut(
                &mk_field!(CustomerTuple, c_payment_cnt),
                self.customer.schema(),
            )
        };
        *c_payment_cnt += 1;

        let c_credit = unsafe {
            c_results.get_value::<FixedString<2>, _>(
                &mk_field!(CustomerTuple, c_credit),
                self.customer.schema(),
            )
        };

        {
            let c_middle = unsafe {
                c_results.get_value::<FixedString<2>, _>(
                    &mk_field!(CustomerTuple, c_middle),
                    self.customer.schema(),
                )
            };
            assert!(c_middle.clone() == "OE");
            let c_discount = unsafe {
                c_results.get_value::<f64, _>(
                    &mk_field!(CustomerTuple, c_discount),
                    self.customer.schema(),
                )
            };
            assert!(*c_discount > 0.0 && *c_discount < 0.5);
        }

        /* Insert to history */
        let history_tuple = HistoryTuple {
            h_amount: input.h_amount,
            h_c_id: match input.c_query {
                CustomerQuery::Id(i) => i as u64,
                _ => 0,
            },
            h_c_d_id: input.d_id as u64,
            h_c_w_id: input.w_id as u64,
            h_d_id: input.d_id as u64,
            h_w_id: input.w_id as u64,
            h_date: input.h_date,
            h_data: FixedString::concat(&w_name, &d_name, 4),
        };

        /* Well, I hate TPCC QAQ */
        let c_data_field = mk_query!(CustomerTuple, c_data);
        if c_credit.clone() == "BC" {
            let c_data_result = self
                .customer
                .read_and_promote(&mut c_lock, &c_data_field)
                .await;

            const SHIFT_AMOUNT: usize = 48; // c_id, c_d_id, c_w_id, d_id, w_id, h_amount -> 48 bytes

            let data: &mut [u8; C_DATA_SIZE] = unsafe {
                c_data_result
                    .get_value_mut(&mk_field!(CustomerTuple, c_data), self.customer.schema())
            };
            let data_ptr = data as *mut [u8; C_DATA_SIZE];
            unsafe {
                std::ptr::copy(
                    data_ptr as *const u8,
                    (data_ptr as *mut [u8; C_DATA_SIZE] as *mut u8).add(SHIFT_AMOUNT),
                    C_DATA_SIZE - SHIFT_AMOUNT,
                );

                std::ptr::copy_nonoverlapping(
                    (&history_tuple) as *const HistoryTuple as *const u8,
                    data_ptr as *mut [u8; C_DATA_SIZE] as *mut u8,
                    SHIFT_AMOUNT,
                );
            }
        };

        // History is in the log now
        // self.history.insert(history_tuple);
        Ok(())
    }
}
