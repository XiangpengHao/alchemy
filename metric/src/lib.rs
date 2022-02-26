use serde::Serialize;
use std::cell::UnsafeCell;
use std::ops::AddAssign;

pub(crate) mod counter;
mod ctx_counter;
pub(crate) mod histogram;
pub mod recorders;
pub mod timer;

pub use crate::counter::Counter;
pub use crate::ctx_counter::CtxCounter;
pub use crate::histogram::Histogram;
pub use crate::timer::Timer;

pub use recorders::TlsRecorder;

thread_local! {
    static LOCAL_RECORDER: UnsafeCell<TlsRecorder> = UnsafeCell::new(TlsRecorder::default());
}

trait RecorderImpl: Serialize + AddAssign + Sized {
    fn reset(&mut self);
}

pub fn get_tls_recorder() -> &'static mut TlsRecorder {
    LOCAL_RECORDER.with(|id| unsafe { &mut *id.get() })
}

#[macro_export]
macro_rules! counter {
    ($event:expr) => {
        if cfg!(feature = "metrics") {
            $crate::get_tls_recorder().increment_counter($event, 1);
        }
    };
    ($event:expr, $value:literal) => {
        if cfg!(feature = "metrics") {
            $crate::get_tls_recorder().increment_counter($event, $value);
        }
    };
    ($event:expr, $tag:expr) => {
        if cfg!(feature = "metrics") {
            match $tag {
                Some(v) => {
                    $crate::get_tls_recorder().ctx_counter($event, 1, v);
                }
                None => {
                    $crate::get_tls_recorder().increment_counter($event, 1);
                }
            }
        }
    };
}

#[macro_export]
macro_rules! histogram {
    ($event:expr, $value:expr) => {
        if cfg!(feature = "metrics") {
            $crate::get_tls_recorder().hit_histogram($event, $value);
        }
    };
}

#[macro_export]
macro_rules! timer {
    ($event:expr) => {
        let _timer_guard = if cfg!(all(feature = "metrics", feature = "latency")) {
            Some($crate::get_tls_recorder().timer_guard($event))
        } else {
            None
        };
    };
}
