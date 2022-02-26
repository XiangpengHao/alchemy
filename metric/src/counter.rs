use crate::RecorderImpl;
use metric_utils::MetricEnum;
use serde::{ser::SerializeStruct, Serialize, Serializer};

#[repr(u8)]
#[derive(Debug, Hash, Eq, PartialEq, MetricEnum)]
pub enum Counter {
    ReadHit = 0,
    ReadMiss = 1,
    ProbeMiss = 2,
    ReadSchemaMiss = 3,
    ReadCnt = 4,
    UpdateCnt = 5,
    InsertCnt = 6,
    UpdateSchemaMiss = 7,
    UpdateHit = 8,
    UpdateMiss = 9,
    AbortRLockBusy = 10,
    AbortWLockBusy = 11,
    AbortUpgradeBusy = 12,
    AbortIdxNotFound = 13,
    AbortLockBusy = 14,
    DeliveryNewOrderNotFound = 15,
    NewOrderRollback = 16,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct CounterRecorder {
    pub(crate) counters: [u64; Counter::LENGTH],
}

impl CounterRecorder {
    pub(crate) fn increment(&mut self, event: Counter, amount: u64) {
        let counter = unsafe { self.counters.get_unchecked_mut(event as usize) };
        *counter += amount;
    }
}

impl Serialize for CounterRecorder {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("counters", self.counters.len())?;
        for i in 0..self.counters.len() {
            match Counter::from_num(i) {
                Counter::ReadHit => {
                    state.serialize_field("read_hit", &self.counters[Counter::ReadHit as usize])?
                }
                Counter::ReadMiss => state
                    .serialize_field("read_miss", &self.counters[Counter::ReadMiss as usize])?,
                Counter::UpdateHit => state
                    .serialize_field("update_hit", &self.counters[Counter::UpdateHit as usize])?,
                Counter::UpdateMiss => state
                    .serialize_field("update_miss", &self.counters[Counter::UpdateMiss as usize])?,
                Counter::ProbeMiss => state
                    .serialize_field("probe_miss", &self.counters[Counter::ProbeMiss as usize])?,
                Counter::ReadSchemaMiss => state.serialize_field(
                    "read_schema_miss",
                    &self.counters[Counter::ReadSchemaMiss as usize],
                )?,
                Counter::UpdateSchemaMiss => state.serialize_field(
                    "update_schema_miss",
                    &self.counters[Counter::UpdateSchemaMiss as usize],
                )?,
                Counter::ReadCnt => {
                    state.serialize_field("read_cnt", &self.counters[Counter::ReadCnt as usize])?
                }
                Counter::UpdateCnt => state
                    .serialize_field("update_cnt", &self.counters[Counter::UpdateCnt as usize])?,
                Counter::InsertCnt => state
                    .serialize_field("insert_cnt", &self.counters[Counter::InsertCnt as usize])?,
                Counter::AbortRLockBusy => state.serialize_field(
                    "abort_read_lock_busy",
                    &self.counters[Counter::AbortRLockBusy as usize],
                )?,
                Counter::AbortWLockBusy => state.serialize_field(
                    "abort_write_lock_busy",
                    &self.counters[Counter::AbortWLockBusy as usize],
                )?,
                Counter::AbortUpgradeBusy => state.serialize_field(
                    "abort_upgrade_lock_busy",
                    &self.counters[Counter::AbortUpgradeBusy as usize],
                )?,
                Counter::AbortIdxNotFound => state.serialize_field(
                    "abort_idx_not_found",
                    &self.counters[Counter::AbortIdxNotFound as usize],
                )?,

                Counter::AbortLockBusy => state.serialize_field(
                    "abort_lock_busy",
                    &self.counters[Counter::AbortLockBusy as usize],
                )?,
                Counter::DeliveryNewOrderNotFound => state.serialize_field(
                    "delivery_new_order_not_found",
                    &self.counters[Counter::DeliveryNewOrderNotFound as usize],
                )?,
                Counter::NewOrderRollback => state.serialize_field(
                    "new_order_rollback",
                    &self.counters[Counter::NewOrderRollback as usize],
                )?,
            }
        }
        state.end()
    }
}

impl RecorderImpl for CounterRecorder {
    fn reset(&mut self) {
        for i in self.counters.iter_mut() {
            *i = 0;
        }
    }
}

use auto_ops::impl_op_ex;

impl_op_ex!(+= |a: &mut CounterRecorder, b: &CounterRecorder| {
    for i in 0..Counter::LENGTH{
        a.counters[i] += b.counters[i];
    }
});

impl_op_ex!(+ |a: &CounterRecorder, b: &CounterRecorder| -> CounterRecorder {
    let mut c_a = a.clone();
    c_a += b;
    c_a
});
