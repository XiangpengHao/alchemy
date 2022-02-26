use std::{
    future::Future,
    task::{Context, Poll},
};

use caching::async_task::{test_utils::*, Executor};
use criterion::{criterion_group, criterion_main, Criterion};

const GROUP_SIZE: usize = 8;

fn linked_list_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("linkedlist");
    group.sample_size(10);
    let array_size = 1024 * 1024;
    let workloads: Vec<ArrayList> = [0; GROUP_SIZE]
        .iter()
        .map(|_| ArrayList::new(array_size as usize))
        .collect();

    let gt_sum = workloads[0].ground_truth_sum() * GROUP_SIZE as u64;

    group.bench_function("async", |b| {
        b.iter(|| {
            let sum = async_traverse(&workloads);
            assert_eq!(sum, gt_sum);
        })
    });

    group.bench_function("sync", |b| {
        b.iter(|| {
            let sum = simple_traverse(&workloads);
            assert_eq!(sum, gt_sum);
        })
    });
}

pub struct SimpleFuture {
    is_first_poll: bool,
}

impl SimpleFuture {
    pub fn new() -> Self {
        SimpleFuture {
            is_first_poll: true,
        }
    }
}

use std::pin::Pin;

impl Future for SimpleFuture {
    type Output = usize;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.is_first_poll {
            self.is_first_poll = false;
            Poll::Pending
        } else {
            Poll::Ready(1)
        }
    }
}

async fn simple_yield() -> usize {
    let mut sum = 0;
    for _i in 0..10000 {
        sum += SimpleFuture::new().await
    }
    sum
}

async fn stackful_yield() -> usize {
    let large_array: [usize; 32] = [42; 32];
    let mut sum = 0;
    for _i in 0..10000 {
        sum += SimpleFuture::new().await
    }
    sum + large_array.iter().sum::<usize>()
}

fn switch_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("yeild");
    println!(
        "size of simple future: {}",
        std::mem::size_of_val(&simple_yield())
    );
    println!(
        "size of heavy future: {}",
        std::mem::size_of_val(&stackful_yield())
    );

    group.bench_function("simple", |b| {
        b.iter(|| {
            let mut executor = Executor::<_, GROUP_SIZE>::new();
            for _i in 0..GROUP_SIZE {
                executor.spawn(simple_yield());
            }
            executor.run_ready_tasks();
        });
    });

    group.bench_function("heavy", |b| {
        b.iter(|| {
            let mut executor = Executor::<_, GROUP_SIZE>::new();
            for _i in 0..GROUP_SIZE {
                executor.spawn(stackful_yield());
            }
            executor.run_ready_tasks();
        });
    });
}

criterion_group!(benches, linked_list_bench, switch_bench);
criterion_main!(benches);
