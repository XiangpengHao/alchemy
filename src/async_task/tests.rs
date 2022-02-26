use crate::async_task::test_utils::*;

const GROUP_SIZE: usize = 8;

#[test]
fn simple_traverse_is_correct() {
    let array_size = 1024;
    let workloads: Vec<ArrayList> = [0; GROUP_SIZE]
        .iter()
        .map(|_| ArrayList::new(array_size as usize))
        .collect();

    let workload_sum = {
        let mut total_sum = 0;
        for workload in workloads.iter() {
            total_sum += workload.ground_truth_sum();
        }
        total_sum
    };

    let sum = simple_traverse(&workloads);
    assert_eq!(sum, workload_sum);
}

#[test]
fn async_yield_same_as_simple() {
    let array_size = 1024;
    let workloads: Vec<ArrayList> = [0; GROUP_SIZE]
        .iter()
        .map(|_| ArrayList::new(array_size as usize))
        .collect();

    let sync_sum = simple_traverse(&workloads);

    let async_sum = async_traverse(&workloads);

    assert_eq!(sync_sum, async_sum);
}
