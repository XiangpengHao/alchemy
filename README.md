# Alchemy
**A**ttribute-**L**evel **C**aching in **He**terogeneous In-**M**emor**y** DBMS

[![alchemy](https://github.com/XiangpengHao/alchemy/actions/workflows/rust.yml/badge.svg)](https://github.com/XiangpengHao/alchemy/actions/workflows/rust.yml)
[![codecov](https://codecov.io/gh/XiangpengHao/clock-cache/branch/master/graph/badge.svg?token=YS06KZ1RYS)](https://codecov.io/gh/XiangpengHao/clock-cache)

Scalable persistent memory (PM) can potentially lower the cost and increase the capacity of main-memory database systems by replacing or complementing DRAM. 
However, modern main-memory database systems often mandate memory bandwidth beyond what commercial PM devices can offer, yet existing DRAM-based caching strategies do not fully utilize PM’s byte-addressability, leading to suboptimal performance.

Alchemy is a DRAM-PM hybrid database engine built from scratch to achieve high performance at a low cost. The key is to leverages PM’s byte-addressability to cache only hot attributes in DRAM, reducing requirements on PM bandwidth. 
To mitigate the higher latency of PM, Alchemy leverages lightweight coroutines to overlap data fetching and computation, effectively hiding CPU stalls. 
Experiments using Intel Optane DCPMM show that Alchemy is up to 2×faster and 2.17×more cost-effective than highly-optimized, traditional DRAM-based designs.


## Run Alchemy

### Prerequisites
- Nightly rust, install here https://rustup.rs
- A linux server with Intel Optane Persistent Memory Module, configured as AppDirect mode and mounted to `/mnt/pmem`
- An NVMe SSD (for logging) should be mounted to `/mnt/ssd`
- [Optional] For more advanced internal metrics, the kernel should enable `msr` module and a `pcm-sensor-server` should be running in the background

### Build
```bash
cargo build . --release
```

### Test

Run unit test
```bash
cargo test
```

To test with sanitizers:
```bash
env ASAN_OPTIONS="detect_odr_violation=0" RUSTFLAGS="-Z sanitizer=address" cargo test -Zbuild-std --target x86_64-unknown-linux-gnu --features asans -- --skip test_alloc_oom
```


### Benchmark
Alchemy has microbenchmarks and TPC-C benchmark.
To run microbenchmark
```bash
cargo run --bin table --release "test-name"
```

To run TPC-C benchmark
```bash
cargo run --bin tpcc --release "tpcc-basic"
```

Alchemy collects comprehensive runtime statistics.
To enable flamegraph support, add `flamegraph` feature;
To enable Alchemy internal metrics, add `metric` feature;
To enable PCM counter support, add `pcm` feature.
For example:
```bash
cargo run --bin table --release "benchmark-name" --features "metric pcm flamegraph"
```

All benchmark results will be saved to a json file.

## Alchemy benchmark results
We conduct experiments on a server equipped with a 16-core (32
hyperthreads) Intel Xeon Gold 5218 CPU clocked at 2.3GHz with
22 MB of on-chip SRAM cache. 
The machine has 32GB of DRAM (two channels, 16GB each) and 512GB of Intel Optane DCPMM (four channels, 128GB each); both DRAM and DCPMM DIMMs are distributed across memory channels to max out the available bandwidth. 

Our experiments run on Ubuntu Linux 18.04 with Linux kernel 5.0.21; 
the source code is implemented and compiled using Rust 1.58. 
The database accesses PM using normal load and store instructions enabled by the mmap interface under the fsdax
mode. 

For each experiment, we run the program for a warmup
period so that the performance numbers stabilize. 
We then measure performance for four runs, with 15-second each, and report the average performance of the four runs. Every experiment has logging enabled, which sends data to an NVMe SSD with a write bandwidth of ∼2.7GB/s; 
the maximum write traffic of all experiments is ∼700MB/s,indicating that logging to SSD is not a bottleneck.

Raw benchmark data can be found in the `experiments/` folder of this repo.


