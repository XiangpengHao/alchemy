use std::{marker::PhantomData, u64, usize};

use crate::{
    mk_field,
    tpcc::tables::{
        CustomerTuple, DistrictTuple, ItemTuple, OrderLineTuple, OrderTuple, StockTuple,
        WarehouseTuple,
    },
    utils::FixedString,
};
use alchemy::{cache_manager::Schema, query::Field};
use memoffset::{offset_of, span_of};

pub struct GeneralSchema<T> {
    cached: PhantomData<T>,
}

impl<F> Default for GeneralSchema<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> GeneralSchema<T> {
    pub fn new() -> Self {
        Self {
            cached: PhantomData,
        }
    }
}

pub struct WarehouseCached {
    ytd: f64,
}

impl Schema for GeneralSchema<WarehouseCached> {
    type Tuple = WarehouseTuple;
    type Field = WarehouseCached;

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        WarehouseCached { ytd: tuple.w_ytd }
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        tuple.w_ytd = field.ytd;
    }

    fn key(&self, tuple: &Self::Tuple) -> usize {
        tuple.w_id as usize
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        let cached = Field::from_span(span_of!(WarehouseTuple, w_ytd));
        if cached == *tuple_field {
            Some(mk_field!(WarehouseCached, ytd))
        } else {
            None
        }
    }
}

impl Schema for GeneralSchema<WarehouseTuple> {
    type Field = WarehouseTuple;
    type Tuple = WarehouseTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        tuple.w_id as usize
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        tuple.clone()
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        *tuple = field.clone();
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        Some(*tuple_field)
    }
}

pub struct DistrictCached {
    ytd: f64,
}

impl Schema for GeneralSchema<DistrictCached> {
    type Tuple = DistrictTuple;
    type Field = DistrictCached;

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        DistrictCached { ytd: tuple.d_ytd }
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        tuple.d_ytd = field.ytd;
    }

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.d_w_id, tuple.d_id)
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        let cached = Field::from_span(span_of!(DistrictTuple, d_ytd));
        if cached == *tuple_field {
            Some(mk_field!(DistrictCached, ytd))
        } else {
            None
        }
    }
}

impl Schema for GeneralSchema<DistrictTuple> {
    type Field = DistrictTuple;
    type Tuple = DistrictTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.d_w_id, tuple.d_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        tuple.clone()
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        *tuple = field.clone();
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        Some(*tuple_field)
    }
}

pub struct CustomerCachedWrite {
    pub c_balance: f64,
    pub c_ytd_payment: f64,
    pub c_payment_cnt: u32,
}

impl Schema for GeneralSchema<CustomerCachedWrite> {
    type Field = CustomerCachedWrite;
    type Tuple = CustomerTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.c_w_id, tuple.c_d_id, tuple.c_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        CustomerCachedWrite {
            c_balance: tuple.c_balance,
            c_ytd_payment: tuple.c_ytd_payment,
            c_payment_cnt: tuple.c_payment_cnt,
        }
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        tuple.c_balance = field.c_balance;
        tuple.c_ytd_payment = field.c_ytd_payment;
        tuple.c_payment_cnt = field.c_payment_cnt;
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        let fields = [
            mk_field!(CustomerTuple, c_balance),
            mk_field!(CustomerTuple, c_ytd_payment),
            mk_field!(CustomerTuple, c_payment_cnt),
        ];
        for (i, f) in fields.iter().enumerate() {
            if f == tuple_field {
                match i {
                    0 => return Some(mk_field!(CustomerCachedWrite, c_balance)),
                    1 => return Some(mk_field!(CustomerCachedWrite, c_ytd_payment)),
                    2 => return Some(mk_field!(CustomerCachedWrite, c_payment_cnt)),
                    _ => unreachable!(),
                }
            }
        }
        None
    }
}

impl Schema for GeneralSchema<CustomerTuple> {
    type Field = CustomerTuple;
    type Tuple = CustomerTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        CustomerTuple::make_key(tuple.c_w_id, tuple.c_d_id, tuple.c_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        tuple.clone()
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        *tuple = field.clone();
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        Some(*tuple_field)
    }
}

#[repr(C)]
pub struct CustomerCachedRead {
    pub c_id: u64,
    pub c_first: FixedString<20>,
    pub c_middle: FixedString<2>,
    pub c_last: FixedString<16>,
    pub c_street1: FixedString<20>,
    pub c_street2: FixedString<20>,
    pub c_city: FixedString<20>,
    pub c_state: FixedString<2>,
    pub c_zip: u64,
    pub c_phone: FixedString<16>,
    pub c_since: u64,
    pub c_credit: FixedString<2>,
    pub c_credit_lim: f64,
    pub c_discount: f64,
    pub c_balance: f64,
    pub c_ytd_payment: f64,
    pub c_payment_cnt: u32,
    pub c_delivery_cnt: u32,
}

impl Schema for GeneralSchema<CustomerCachedRead> {
    type Field = CustomerCachedRead;
    type Tuple = CustomerTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.c_w_id, tuple.c_d_id, tuple.c_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        CustomerCachedRead {
            c_id: tuple.c_id,
            c_first: tuple.c_first.clone(),
            c_middle: tuple.c_middle.clone(),
            c_last: tuple.c_last.clone(),
            c_street1: tuple.c_street1.clone(),
            c_street2: tuple.c_street2.clone(),
            c_city: tuple.c_city.clone(),
            c_state: tuple.c_state.clone(),
            c_zip: tuple.c_zip,
            c_phone: tuple.c_phone.clone(),
            c_since: tuple.c_since,
            c_credit: tuple.c_credit.clone(),
            c_credit_lim: tuple.c_credit_lim,
            c_discount: tuple.c_discount,
            c_balance: tuple.c_balance,
            c_ytd_payment: tuple.c_ytd_payment,
            c_payment_cnt: tuple.c_payment_cnt,
            c_delivery_cnt: tuple.c_delivery_cnt,
        }
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        tuple.c_id = field.c_id;
        tuple.c_first = field.c_first.clone();
        tuple.c_middle = field.c_middle.clone();
        tuple.c_last = field.c_last.clone();
        tuple.c_street1 = field.c_street1.clone();
        tuple.c_street2 = field.c_street2.clone();
        tuple.c_city = field.c_city.clone();
        tuple.c_state = field.c_state.clone();
        tuple.c_zip = field.c_zip;
        tuple.c_phone = field.c_phone.clone();
        tuple.c_since = field.c_since;
        tuple.c_credit = field.c_credit.clone();
        tuple.c_credit_lim = field.c_credit_lim;
        tuple.c_discount = field.c_discount;
        tuple.c_balance = field.c_balance;
        tuple.c_ytd_payment = field.c_ytd_payment;
        tuple.c_payment_cnt = field.c_payment_cnt;
        tuple.c_delivery_cnt = field.c_delivery_cnt;
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        const END_OFFSET: usize = offset_of!(CustomerTuple, c_w_id);

        if (tuple_field.begin() as usize) >= END_OFFSET {
            None
        } else {
            Some(*tuple_field)
        }
    }
}

impl Schema for GeneralSchema<ItemTuple> {
    type Field = ItemTuple;
    type Tuple = ItemTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.i_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        tuple.clone()
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        *tuple = field.clone()
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        Some(*tuple_field)
    }
}

#[repr(C)]
pub struct ItemCached {
    pub(crate) i_price: f64,
    pub(crate) i_name: FixedString<24>,
    pub(crate) i_data: FixedString<50>,
}

impl Schema for GeneralSchema<ItemCached> {
    type Field = ItemCached;
    type Tuple = ItemTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.i_id)
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        tuple.i_price = field.i_price;
        tuple.i_name = field.i_name.clone();
        tuple.i_data = field.i_data.clone();
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        const BEGIN_OFFSET: usize = offset_of!(ItemTuple, i_price);

        if (tuple_field.begin() as usize) < BEGIN_OFFSET {
            None
        } else {
            Some(Field::new(
                tuple_field.begin() - BEGIN_OFFSET as u16,
                tuple_field.end() - BEGIN_OFFSET as u16,
            ))
        }
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        ItemCached {
            i_price: tuple.i_price,
            i_name: tuple.i_name.clone(),
            i_data: tuple.i_data.clone(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct OrderCached {
    pub(crate) o_id: u64,
    pub(crate) o_c_id: u64,
    pub(crate) o_entry_d: u64,
    pub(crate) o_carrier_id: u64,
}

impl Schema for GeneralSchema<OrderCached> {
    type Field = OrderCached;
    type Tuple = OrderTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.o_w_id, tuple.o_d_id, tuple.o_c_id, tuple.o_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        OrderCached {
            o_id: tuple.o_id,
            o_c_id: tuple.o_c_id,
            o_entry_d: tuple.o_entry_d,
            o_carrier_id: tuple.o_carrier_id,
        }
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        tuple.o_id = field.o_id;
        tuple.o_c_id = field.o_c_id;
        tuple.o_entry_d = field.o_entry_d;
        tuple.o_carrier_id = field.o_carrier_id;
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        const END_OFFSET: usize = offset_of!(OrderTuple, o_d_id);
        if (tuple_field.begin() as usize) >= END_OFFSET {
            None
        } else {
            Some(*tuple_field)
        }
    }
}

impl Schema for GeneralSchema<OrderTuple> {
    type Field = OrderTuple;
    type Tuple = OrderTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.o_w_id, tuple.o_d_id, tuple.o_c_id, tuple.o_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        tuple.clone()
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        *tuple = field.clone()
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        Some(*tuple_field)
    }
}

#[repr(C)]
pub struct OrderLineCached {
    pub(crate) ol_o_id: u64,
    pub(crate) ol_d_id: u64,
    pub(crate) ol_i_id: u64,
    pub(crate) ol_supply_w_id: u64,
    pub(crate) ol_quantity: u64,
    pub(crate) ol_delivery_d: u64,
    pub(crate) ol_amount: f64,
}

impl Schema for GeneralSchema<OrderLineCached> {
    type Field = OrderLineCached;
    type Tuple = OrderLineTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(
            tuple.ol_w_id as usize,
            tuple.ol_d_id as usize,
            tuple.ol_o_id as usize,
            tuple.ol_i_id as usize,
        )
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        OrderLineCached {
            ol_o_id: tuple.ol_o_id,
            ol_d_id: tuple.ol_d_id,
            ol_i_id: tuple.ol_i_id,
            ol_supply_w_id: tuple.ol_supply_w_id,
            ol_quantity: tuple.ol_quantity,
            ol_delivery_d: tuple.ol_delivery_d,
            ol_amount: tuple.ol_amount,
        }
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        tuple.ol_o_id = field.ol_o_id;
        tuple.ol_d_id = field.ol_d_id;
        tuple.ol_i_id = field.ol_i_id;
        tuple.ol_supply_w_id = field.ol_supply_w_id;
        tuple.ol_quantity = field.ol_quantity;
        tuple.ol_delivery_d = field.ol_delivery_d;
        tuple.ol_amount = field.ol_amount;
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        const END_OFFSET: usize = offset_of!(OrderLineTuple, ol_number);
        if (tuple_field.begin() as usize) >= END_OFFSET {
            None
        } else {
            Some(*tuple_field)
        }
    }
}

impl Schema for GeneralSchema<OrderLineTuple> {
    type Field = OrderLineTuple;
    type Tuple = OrderLineTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(
            tuple.ol_w_id as usize,
            tuple.ol_d_id as usize,
            tuple.ol_o_id as usize,
            tuple.ol_i_id as usize,
        )
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        tuple.clone()
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        *tuple = field.clone()
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        Some(*tuple_field)
    }
}

impl Schema for GeneralSchema<StockTuple> {
    type Field = StockTuple;
    type Tuple = StockTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.s_w_id, tuple.s_i_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        tuple.clone()
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        *tuple = field.clone()
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        Some(*tuple_field)
    }
}

#[repr(C)]
pub struct StockCached {
    s_i_id: u64,
    s_w_id: u64,
    s_quantity: u64,
    s_ytd: u64,
    s_order_cnt: u64,
    s_remote_cnt: u64,
    s_data: FixedString<24>,
    s_dist10: FixedString<24>,
}

impl Schema for GeneralSchema<StockCached> {
    type Field = StockCached;
    type Tuple = StockTuple;

    fn key(&self, tuple: &Self::Tuple) -> usize {
        Self::Tuple::make_key(tuple.s_w_id, tuple.s_i_id)
    }

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        StockCached {
            s_i_id: tuple.s_i_id,
            s_w_id: tuple.s_w_id,
            s_quantity: tuple.s_quantity,
            s_ytd: tuple.s_ytd,
            s_order_cnt: tuple.s_order_cnt,
            s_remote_cnt: tuple.s_remote_cnt,
            s_data: tuple.s_data.clone(),
            s_dist10: tuple.s_dist10.clone(),
        }
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        const END_OFFSET: usize = offset_of!(StockTuple, s_dist01);

        if *tuple_field == mk_field!(StockTuple, s_dist10) {
            return Some(mk_field!(StockCached, s_dist10));
        }

        if (tuple_field.begin() as usize) >= END_OFFSET {
            None
        } else {
            Some(*tuple_field)
        }
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        tuple.s_quantity = field.s_quantity;
        tuple.s_ytd = field.s_ytd;
        tuple.s_order_cnt = field.s_order_cnt;
        tuple.s_remote_cnt = field.s_remote_cnt;
        tuple.s_data = field.s_data.clone();
    }
}
