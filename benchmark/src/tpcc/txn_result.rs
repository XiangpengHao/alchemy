use std::{
    fmt::{self, Display},
    iter::Sum,
    time::Duration,
};

use serde::Serialize;

pub type MetricResult = metric::TlsRecorder;

#[derive(Clone, Serialize, Default)]
pub struct TransactionResult {
    pub commit: usize,
    pub abort: usize,
}

impl TransactionResult {
    pub fn new(commit: usize, abort: usize) -> TransactionResult {
        Self { commit, abort }
    }

    pub fn inc_commit(&mut self) {
        self.commit += 1;
    }

    pub fn inc_abort(&mut self) {
        self.abort += 1;
    }

    pub fn get_commit(&self) -> usize {
        self.commit
    }
}

impl Display for TransactionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "commit {} op/s, abort {} op/s", self.commit, self.abort)
    }
}

impl_op_ex!(+= |a: &mut TransactionResult, b: &TransactionResult| {
    a.commit += b.commit;
    a.abort += b.abort;
});

impl_op_ex!(+ |a: &TransactionResult, b: &TransactionResult| -> TransactionResult{
    let mut c_a = a.clone();
    c_a += b;
    c_a
});

impl<'a> Sum<&'a Self> for TransactionResult {
    fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
        iter.fold(TransactionResult::default(), |h, v| h + v.clone())
    }
}

impl shumai::BenchResult for TransactionResult {
    fn short_value(&self) -> usize {
        self.commit
    }

    fn normalize_time(self, dur: &Duration) -> Self {
        Self {
            commit: (self.commit as f64 / dur.as_secs_f64()) as usize,
            abort: (self.abort as f64 / dur.as_secs_f64()) as usize,
        }
    }
}

#[derive(Clone, Serialize, Default)]
pub struct TpccResults {
    pub payment: TransactionResult,
    pub new_order: TransactionResult,
    pub order_status: TransactionResult,
    pub delivery: TransactionResult,
    pub stock_level: TransactionResult,
    metrics: Option<MetricResult>,
}

impl TpccResults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_metric(&mut self, metric: MetricResult) {
        self.metrics = Some(metric);
    }
}

use auto_ops::impl_op_ex;
use shumai::BenchResult;
impl_op_ex!(+= |a: &mut TpccResults, b: &TpccResults| {
    a.payment += &b.payment;
    a.new_order += &b.new_order;
    a.order_status += &b.order_status;
    a.delivery += &b.delivery;
    a.stock_level += &b.stock_level;

    if let Some(v) = &b.metrics {
        if let Some(a_m) = &a.metrics{
            a.metrics = Some(a_m + v);
        } else {
            a.metrics = Some(v.clone());
        }
    }
});

impl_op_ex!(+ |a: &TpccResults, b: &TpccResults| -> TpccResults{
    let mut c_a = a.clone();
    c_a += b;
    c_a
});

impl Display for TpccResults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "New Order:\t{}\nPayment:\t{}\nOrderStatus:\t{}\nStockLevel:\t{}\nDelivery:\t{}",
            self.new_order, self.payment, self.order_status, self.stock_level, self.delivery
        )
    }
}

impl<'a> Sum<&'a Self> for TpccResults {
    fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
        iter.fold(TpccResults::default(), |h, v| h + v.clone())
    }
}

impl BenchResult for TpccResults {
    fn short_value(&self) -> usize {
        self.new_order.short_value()
            + self.payment.short_value()
            + self.order_status.short_value()
            + self.delivery.short_value()
            + self.stock_level.short_value()
    }

    fn normalize_time(self, dur: &Duration) -> Self {
        Self {
            new_order: self.new_order.normalize_time(dur),
            payment: self.payment.normalize_time(dur),
            order_status: self.order_status.normalize_time(dur),
            delivery: self.delivery.normalize_time(dur),
            stock_level: self.stock_level.normalize_time(dur),
            metrics: self.metrics,
        }
    }
}
