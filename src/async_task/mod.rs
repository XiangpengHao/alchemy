use std::{
    future::Future,
    mem::{self, MaybeUninit},
    pin::Pin,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

mod prefetcher;
pub(crate) mod task_switcher;
pub use prefetcher::{PrefetchRef, Prefetcher};
pub use task_switcher::DummySwitcher;

#[cfg(test)]
mod tests;

#[doc(hidden)]
#[cfg(test)]
mod test_utils;

/// An executor that only run one task
pub fn block_on<F: Future>(mut task: F) -> F::Output {
    let waker = dummy_waker();
    let mut context = Context::from_waker(&waker);

    loop {
        let pinned_task = unsafe { Pin::new_unchecked(&mut task) };
        if let Poll::Ready(out) = pinned_task.poll(&mut context) {
            return out;
        }
    }
}

pub struct Executor<F: Future, const N: usize> {
    task_queue: [Option<F>; N],
}

impl<F: Future, const N: usize> Default for Executor<F, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Future, const N: usize> Executor<F, N> {
    pub fn new() -> Self {
        let mut queue: [MaybeUninit<Option<F>>; N] =
            unsafe { std::mem::MaybeUninit::uninit().assume_init() };
        for elem in queue.iter_mut() {
            unsafe { std::ptr::write(elem.as_mut_ptr(), None) };
        }

        // We should use transmute instead, look at this: https://github.com/rust-lang/rust/issues/61956
        let queue_rv = unsafe {
            (&mut queue as *mut [MaybeUninit<Option<F>>; N] as *mut [Option<F>; N]).read()
        };
        Executor {
            task_queue: queue_rv,
        }
    }

    pub fn spawn(&mut self, task: F) {
        for i in 0..N {
            if self.task_queue[i].is_none() {
                self.task_queue[i] = Some(task);
                return;
            }
        }
        panic!("max executor queue reached!");
    }

    pub fn run_ready_tasks(&mut self) -> [F::Output; N] {
        let mut pos = 0;
        let mut ready_task: u8 = 0;
        let mut output: [MaybeUninit<F::Output>; N] =
            unsafe { MaybeUninit::uninit().assume_init() };

        let waker = dummy_waker();
        let mut context = Context::from_waker(&waker);

        loop {
            if let Some(task) = self.task_queue[pos as usize].as_mut() {
                let pinned_task = unsafe { Pin::new_unchecked(task) };
                if let Poll::Ready(sum) = pinned_task.poll(&mut context) {
                    ready_task += 1;
                    output[pos] = MaybeUninit::new(sum);
                    self.task_queue[pos] = None;
                }
            }
            pos += 1;

            if ready_task == N as u8 {
                let ret = unsafe { mem::transmute_copy(&output) };
                mem::forget(output);
                return ret;
            }

            pos %= N;
        }
    }
}

fn dummy_raw_waker() -> RawWaker {
    fn no_op(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker {
        dummy_raw_waker()
    }
    let vtable = &RawWakerVTable::new(clone, no_op, no_op, no_op);
    RawWaker::new(std::ptr::null() as *const (), vtable)
}

fn dummy_waker() -> Waker {
    unsafe { Waker::from_raw(dummy_raw_waker()) }
}
