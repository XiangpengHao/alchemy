use core::panic;
use rand::{prelude::ThreadRng, Rng};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TxnEngine {
    Sync,
    Async,
}

impl fmt::Display for TxnEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            TxnEngine::Sync => write!(f, "sync"),
            TxnEngine::Async => write!(f, "async"),
        }
    }
}

/// In the future we may support all of the queries,
/// now we only do two
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WorkloadMix {
    new_order: u64,
    payment: u64,
    order_status: u64,
    delivery: u64,
    stock_level: u64,
}

impl fmt::Display for WorkloadMix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "({}-{}-{}-{}-{})",
            self.new_order, self.payment, self.order_status, self.delivery, self.stock_level
        )
    }
}

pub enum Workload {
    NewOrder,
    Payment,
    OrderStatus,
    Delivery,
    StockLevel,
}

impl WorkloadMix {
    pub fn gen(&self, rng: &mut ThreadRng) -> Workload {
        let w = rng.gen_range(0..100);

        let new_order = self.new_order;
        let payment = new_order + self.payment;
        let order_status = payment + self.order_status;
        let delivery = order_status + self.delivery;
        let stock_level = delivery + self.stock_level;

        if w < new_order {
            Workload::NewOrder
        } else if w < payment {
            Workload::Payment
        } else if w < order_status {
            Workload::OrderStatus
        } else if w < delivery {
            Workload::Delivery
        } else if w < stock_level {
            Workload::StockLevel
        } else {
            panic!("workload not supported!")
        }
    }
}

#[shumai::config(path = "tpcc.toml")]
pub struct Tpcc {
    pub name: String,
    pub time: usize,
    pub threads: Vec<usize>,
    pub warehouse_cnt: usize,
    #[matrix]
    pub policy: crate::CachePolicy,
    #[matrix]
    pub txn_engine: TxnEngine,
    pub workload: WorkloadMix,
}
