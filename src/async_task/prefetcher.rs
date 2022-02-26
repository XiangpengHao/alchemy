use std::{
    future::Future,
    ops::Deref,
    pin::Pin,
    task::{Context, Poll},
};

use crate::utils;

const CACHE_LINE_SIZE: usize = 64;

pub struct Prefetcher {
    is_first_poll: bool,
}

impl Prefetcher {
    #[must_use]
    pub fn fetch_ptr(addr: *const i8) -> Self {
        utils::prefetch::mem_prefetch(addr);

        Prefetcher {
            is_first_poll: true,
        }
    }

    #[must_use]
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn fetch_ptr_n(addr: *const i8, size: usize) -> Self {
        for i in (0..size).step_by(CACHE_LINE_SIZE) {
            utils::prefetch::mem_prefetch(unsafe { addr.add(i) });
        }

        Prefetcher {
            is_first_poll: true,
        }
    }

    #[must_use]
    pub fn fetch<T>(reference: &T) -> Self {
        let size = std::mem::size_of::<T>();
        let ptr = reference as *const T as *const i8;
        Self::fetch_ptr_n(ptr, size)
    }
}

impl Future for Prefetcher {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.is_first_poll {
            self.is_first_poll = false;
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

pub struct PrefetchRef<'a, T> {
    value: &'a T,
    fetched: bool,
}

impl<'a, T> PrefetchRef<'a, T> {
    pub fn new(reference: &'a T) -> Self {
        Self {
            value: reference,
            fetched: false,
        }
    }

    pub fn to_ref(&self) -> &'a T {
        self.value
    }
}

impl<'a, T> Future for PrefetchRef<'a, T> {
    type Output = &'a T;
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.fetched {
            self.fetched = true;
            Poll::Pending
        } else {
            Poll::Ready(self.value)
        }
    }
}

impl<T> Deref for PrefetchRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}
