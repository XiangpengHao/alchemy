use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

pub struct DummySwitcher {
    remaining: usize,
}

impl DummySwitcher {
    pub fn new(round: usize) -> Self {
        DummySwitcher { remaining: round }
    }
}

impl Future for DummySwitcher {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.remaining == 0 {
            Poll::Ready(())
        } else {
            self.remaining -= 1;
            Poll::Pending
        }
    }
}
