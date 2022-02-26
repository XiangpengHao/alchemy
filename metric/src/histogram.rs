use crate::RecorderImpl;
use metric_utils::MetricEnum;
use serde::{ser::SerializeStruct, Serialize, Serializer};
use std::collections::HashMap;

#[repr(u8)]
#[derive(Debug, MetricEnum)]
pub enum Histogram {
    ProbeLen = 0,
    Hotness = 1,
    Scanned = 2,
    PaymentCustomerScan = 3,
    OSOrderScan = 4,
    OrderLineScan = 5,
    StockLevelQuantity = 6,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct HistogramRecorder {
    histograms: [HashMap<u16, usize>; Histogram::LENGTH],
}

impl HistogramRecorder {
    pub(crate) fn hit(&mut self, event: Histogram, key: u64) {
        let hist = unsafe { self.histograms.get_unchecked_mut(event as usize) };
        if let Some(v) = hist.get_mut(&(key as u16)) {
            *v += 1;
        } else {
            hist.insert(key as u16, 1);
        }
    }
}

impl Serialize for HistogramRecorder {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("histograms", self.histograms.len())?;
        for (i, h) in self.histograms.iter().enumerate() {
            match Histogram::from_num(i) {
                Histogram::ProbeLen => state.serialize_field("prob_len", h)?,
                Histogram::Hotness => state.serialize_field("hotness", h)?,
                Histogram::Scanned => state.serialize_field("scanned", h)?,
                Histogram::PaymentCustomerScan => {
                    state.serialize_field("payment_customer_scan", h)?
                }
                Histogram::OSOrderScan => state.serialize_field("os_order_scan", h)?,
                Histogram::OrderLineScan => state.serialize_field("order_line_scan", h)?,
                Histogram::StockLevelQuantity => {
                    state.serialize_field("stock_level_quantity", h)?
                }
            }
        }
        state.end()
    }
}

use auto_ops::impl_op_ex;

impl_op_ex!(+= |a: &mut HistogramRecorder, b: &HistogramRecorder| {
    for (i, h) in b.histograms.iter().enumerate(){
        for(k, v) in h.iter(){
            if let Some(value) = a.histograms[i].get_mut(k){
                *value += v;
            }else {
                a.histograms[i].insert(*k, *v);
            }
        }
    }
});

impl_op_ex!(+ |a: &HistogramRecorder, b: &HistogramRecorder| -> HistogramRecorder{
    let mut c_a = a.clone();
    c_a += b;
    c_a
});

impl RecorderImpl for HistogramRecorder {
    fn reset(&mut self) {
        for h in self.histograms.iter_mut() {
            h.clear();
        }
    }
}
