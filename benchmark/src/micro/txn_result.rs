use std::{fmt::Display, iter::Sum, time::Duration};

use serde::Serialize;

pub type MetricResult = metric::TlsRecorder;

#[derive(Clone, Serialize, Default)]
pub struct TransactionResult {
    pub commit: usize,
    pub abort: usize,
    pub metrics: Option<MetricResult>,
}

impl TransactionResult {
    pub fn new(commit: usize, abort: usize) -> TransactionResult {
        Self {
            commit,
            abort,
            metrics: None,
        }
    }

    pub fn inc_commit(&mut self) {
        self.commit += 1;
    }

    pub fn inc_abort(&mut self) {
        self.abort += 1;
    }

    pub fn set_metric(&mut self, metric: MetricResult) {
        self.metrics = Some(metric);
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

use auto_ops::impl_op_ex;

impl_op_ex!(+= |a: &mut TransactionResult, b: &TransactionResult| {
    a.commit += b.commit;
    a.abort += b.abort;

    if let Some(v) = &b.metrics {
        if let Some(a_m) = &a.metrics{
            a.metrics = Some(a_m + v);
        } else {
            a.metrics = Some(v.clone());
        }
    }
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
            metrics: self.metrics,
        }
    }
}
