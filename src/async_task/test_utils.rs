use super::{prefetcher::Prefetcher, Executor};
use rand::seq::SliceRandom;

const GROUP_SIZE: usize = 8;

#[derive(Debug, Clone)]
pub struct Cell {
    next_index: u64,
    _padding: [u64; 7],
}

impl Cell {
    pub fn new(next_idx: u64) -> Self {
        Cell {
            next_index: next_idx,
            _padding: [0; 7],
        }
    }
    pub fn set(&mut self, value: u64) {
        self.next_index = value;
    }

    pub fn get(&self) -> u64 {
        self.next_index
    }
}

pub struct ArrayList {
    pub list: Vec<Cell>,
}

impl ArrayList {
    pub fn new(array_size: usize) -> Self {
        let mut workload_list = ArrayList {
            list: vec![Cell::new(0); array_size],
        };
        let mut temp_values: Vec<u64> = Vec::with_capacity(array_size - 1);
        for i in 1..array_size {
            temp_values.push(i as u64);
        }

        temp_values.shuffle(&mut rand::thread_rng());

        let mut pre_idx = 0;
        for elem in temp_values.iter() {
            workload_list.list[pre_idx].set(*elem);
            pre_idx = *elem as usize;
        }
        workload_list
    }

    pub fn ground_truth_sum(&self) -> u64 {
        ((self.list.len() - 1) * self.list.len() / 2) as u64
    }
}

pub fn simple_traverse(workloads: &[ArrayList]) -> u64 {
    let mut sum: u64 = 0;

    for workload in workloads.iter() {
        let mut pre_idx = 0;
        for _i in 0..workload.list.len() {
            let value = workload.list[pre_idx].get();
            pre_idx = value as usize;
            sum += value;
        }
    }
    sum
}

pub fn async_traverse(workloads: &[ArrayList]) -> u64 {
    let mut executor = Executor::<_, GROUP_SIZE>::new();
    for workload in workloads.iter() {
        executor.spawn(traverse_one(workload));
    }
    executor.run_ready_tasks().iter().sum()
}

async fn traverse_one(workload: &ArrayList) -> u64 {
    let mut pre_idx: usize = 0;
    let mut sum: u64 = 0;

    for _i in 0..workload.list.len() {
        Prefetcher::fetch_ptr(&workload.list[pre_idx] as *const Cell as *const i8).await;
        let value = workload.list[pre_idx].get();
        pre_idx = value as usize;
        sum += value;
    }
    sum
}
