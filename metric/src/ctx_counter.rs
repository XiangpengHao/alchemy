use crate::{counter::CounterRecorder, Counter, RecorderImpl};
use metric_utils::MetricEnum;
use serde::{ser::SerializeMap, Serialize, Serializer};
use std::collections::HashMap;

#[repr(u8)]
#[derive(Debug, MetricEnum, Serialize, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CtxCounter {
    Customer = 2,
    Stock = 3,
    Order = 4,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct CtxCounterRecorder {
    counters: HashMap<CtxCounter, CounterRecorder>,
}

impl CtxCounterRecorder {
    pub(crate) fn increment(&mut self, event: Counter, amount: u64, tag: CtxCounter) {
        match self.counters.get_mut(&tag) {
            Some(c) => {
                let value = unsafe { c.counters.get_unchecked_mut(event as usize) };
                *value += amount;
            }
            None => {
                let mut recorder = CounterRecorder::default();
                let value = unsafe { recorder.counters.get_unchecked_mut(event as usize) };
                *value += amount;
                self.counters.insert(tag, recorder);
            }
        };
    }
}

impl Serialize for CtxCounterRecorder {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_map(Some(self.counters.len()))?;
        for (k, v) in self.counters.iter() {
            state.serialize_entry(k, v)?;
        }
        state.end()
    }
}

use auto_ops::impl_op_ex;

impl_op_ex!(+= |a: &mut CtxCounterRecorder, b: &CtxCounterRecorder| {
    for(rk, rv) in b.counters.iter() {
        match a.counters.get_mut(rk){
            Some(v) => *v += rv,
            None=>{
                a.counters.insert(*rk, rv.clone());
            }
        }
    }
});

impl_op_ex!(+ |a: &CtxCounterRecorder, b: &CtxCounterRecorder| -> CtxCounterRecorder{
    let mut c_a = a.clone();
    c_a += b;
    c_a
});

impl RecorderImpl for CtxCounterRecorder {
    fn reset(&mut self) {
        self.counters.clear();
    }
}
