pub mod config;
pub mod query;
pub mod schemas;
pub mod tables;
mod txn_result;
pub mod workload;

pub use txn_result::TpccResults;

extern crate memoffset;

const DISTRICT_SCALE: usize = 10;
const CUSTOMER_SCALE: usize = 3_000;
const ITEM_SCALE: usize = 100_000;
const CARRIER_NULL: usize = 0;
