use core::f64;
use std::u64;

use alchemy::Tuple;

use crate::utils::FixedString;

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct WarehouseTuple {
    pub w_id: u64,
    pub w_name: FixedString<10>,
    pub w_street1: FixedString<20>,
    pub w_street2: FixedString<20>,
    pub w_city: FixedString<20>,
    pub w_state: FixedString<2>,
    pub w_zip: u64,
    pub w_tax: f64,
    pub w_ytd: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct DistrictTuple {
    pub d_id: u64,
    pub d_w_id: u64,
    pub d_name: FixedString<10>,
    pub d_street1: FixedString<20>,
    pub d_street2: FixedString<20>,
    pub d_city: FixedString<20>,
    pub d_state: FixedString<2>,
    pub d_zip: u64,
    pub d_tax: f64,
    pub d_ytd: f64,
    pub d_next_o_id: u64,
}

impl DistrictTuple {
    pub(crate) fn make_key(w_id: u64, d_id: u64) -> usize {
        ((w_id << 48) + d_id) as usize
    }
}

pub(crate) const C_DATA_SIZE: usize = 500;

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct CustomerTuple {
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
    pub c_w_id: u64,
    pub c_d_id: u64,
    pub c_data: FixedString<C_DATA_SIZE>,
}

impl CustomerTuple {
    pub(crate) fn make_key(w_id: u64, d_id: u64, c_id: u64) -> usize {
        ((w_id << 48) + (d_id << 32) + c_id) as usize
    }

    pub(crate) fn idx_key_lastname(w_id: u16, d_id: u16, last_name: u16, c_id: u16) -> usize {
        let v = ((w_id as u64) << 48)
            + ((d_id as u64) << 32)
            + ((last_name as u64) << 16)
            + (c_id as u64);
        v as usize
    }
}

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct HistoryTuple {
    pub(crate) h_c_id: u64,
    pub(crate) h_c_d_id: u64,
    pub(crate) h_c_w_id: u64,
    pub(crate) h_d_id: u64,
    pub(crate) h_data: FixedString<24>,
    pub(crate) h_w_id: u64,
    pub(crate) h_date: u64,
    pub(crate) h_amount: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct OrderTuple {
    pub(crate) o_id: u64,
    pub(crate) o_c_id: u64,
    pub(crate) o_entry_d: u64,
    pub(crate) o_carrier_id: u64,
    pub(crate) o_d_id: u64,
    pub(crate) o_w_id: u64,
    pub(crate) o_ol_cnt: u64,
    pub(crate) o_all_local: u64,
}

impl OrderTuple {
    pub fn make_key(o_w_id: u64, o_d_id: u64, o_c_id: u64, o_id: u64) -> usize {
        ((o_w_id << 48) + (o_d_id << 32) + (o_c_id << 16) + o_id) as usize
    }
}

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct NewOrderTuple {
    pub(crate) no_o_id: u64,
    pub(crate) no_d_id: u64,
    pub(crate) no_w_id: u64,
}

impl NewOrderTuple {
    pub fn make_key(no_w_id: u64, no_d_id: u64, no_o_id: u64, c_id: u64) -> usize {
        ((no_w_id << 48) + (no_d_id << 32) + (no_o_id << 16) + c_id) as usize
    }

    pub fn o_c_id_from_key(key: usize) -> (usize, usize) {
        let o_id = (key & 0x0000_0000_ffff_0000) >> 16;
        let c_id = key & 0x0000_0000_0000_ffff;
        (o_id, c_id)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct OrderLineTuple {
    pub(crate) ol_o_id: u64,
    pub(crate) ol_d_id: u64,
    pub(crate) ol_i_id: u64,
    pub(crate) ol_supply_w_id: u64,
    pub(crate) ol_quantity: u64,
    pub(crate) ol_delivery_d: u64,
    pub(crate) ol_amount: f64,
    pub(crate) ol_number: u64,
    pub(crate) ol_w_id: u64,
    pub(crate) ol_dist_info: FixedString<24>,
}

impl OrderLineTuple {
    pub fn make_key(ol_w_id: usize, ol_d_id: usize, ol_o_id: usize, ol_i_id: usize) -> usize {
        // The ol_i_id is more than 16 bits
        (ol_w_id << 48) + (ol_d_id << 40) + (ol_o_id << 20) + ol_i_id
    }
}

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct ItemTuple {
    pub(crate) i_id: u64,
    pub(crate) i_im_id: u64,
    pub(crate) i_price: f64,
    pub(crate) i_name: FixedString<24>,
    pub(crate) i_data: FixedString<50>,
}

impl ItemTuple {
    pub fn make_key(i_id: u64) -> usize {
        i_id as usize
    }
}

#[repr(C)]
#[derive(Debug, Clone, Tuple)]
pub struct StockTuple {
    pub(crate) s_i_id: u64,
    pub(crate) s_w_id: u64,
    pub(crate) s_quantity: u64,
    pub(crate) s_ytd: u64,
    pub(crate) s_order_cnt: u64,
    pub(crate) s_remote_cnt: u64,
    pub(crate) s_data: FixedString<24>,
    pub(crate) s_dist01: FixedString<24>,
    pub(crate) s_dist02: FixedString<24>,
    pub(crate) s_dist03: FixedString<24>,
    pub(crate) s_dist04: FixedString<24>,
    pub(crate) s_dist05: FixedString<24>,
    pub(crate) s_dist06: FixedString<24>,
    pub(crate) s_dist07: FixedString<24>,
    pub(crate) s_dist08: FixedString<24>,
    pub(crate) s_dist09: FixedString<24>,
    pub(crate) s_dist10: FixedString<24>,
}

impl StockTuple {
    pub fn make_key(s_w_id: u64, s_i_id: u64) -> usize {
        ((s_w_id << 32) + s_i_id) as usize
    }
}
