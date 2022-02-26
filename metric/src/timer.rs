use crate::RecorderImpl;
use metric_utils::MetricEnum;
use serde::{ser::SerializeStruct, Serialize, Serializer};
use std::time::Duration;
use std::{collections::VecDeque, time::Instant};

#[repr(u8)]
#[derive(Clone, Debug, MetricEnum)]
pub enum Timer {
    Read = 0,
    Tpcc = 1,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct TimerRecorder {
    timers: [VecDeque<u64>; Timer::LENGTH],
}

impl TimerRecorder {
    pub fn start(&self, event: Timer) -> TimerGuard {
        TimerGuard::new(event)
    }

    pub(crate) fn add_time(&mut self, event: Timer, time: Duration) {
        unsafe {
            self.timers
                .get_unchecked_mut(event as usize)
                .push_back(time.as_nanos() as u64);
        }
    }
}

pub struct TimerGuard {
    event: Timer,
    start: Instant,
}

impl TimerGuard {
    fn new(event: Timer) -> Self {
        TimerGuard {
            event,
            start: Instant::now(),
        }
    }
}

use rand::Rng;

impl Drop for TimerGuard {
    fn drop(&mut self) {
        let elapsed = Instant::now() - self.start;

        // sample 0.1% of operations
        let should_sample = rand::thread_rng().gen_bool(0.001);
        if should_sample {
            crate::get_tls_recorder().add_time(self.event.clone(), elapsed);
        }
    }
}

impl RecorderImpl for TimerRecorder {
    fn reset(&mut self) {
        for timer in self.timers.iter_mut() {
            timer.clear();
        }
    }
}

impl Serialize for TimerRecorder {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("timers", self.timers.len())?;
        for i in 0..self.timers.len() {
            match Timer::from_num(i) {
                Timer::Read => state.serialize_field("read", &self.timers[i])?,
                Timer::Tpcc => state.serialize_field("tpcc", &self.timers[i])?,
            }
        }
        state.end()
    }
}

use auto_ops::impl_op_ex;

impl_op_ex!(+= |a: &mut TimerRecorder, b: &TimerRecorder| {
    for(i, t) in a.timers.iter_mut().enumerate(){
        t.extend(b.timers[i].iter());
    }
});

impl_op_ex!(+ |a: &TimerRecorder, b: &TimerRecorder| -> TimerRecorder{
    let mut c_a = a.clone();
    c_a += b;
    c_a
});

// impl AddAssign for TimerRecorder {
//     fn add_assign(&mut self, mut rhs: TimerRecorder) {
//         for (i, t) in self.timers.iter_mut().enumerate() {
//             t.append(&mut rhs.timers[i]);
//         }
//     }
// }
