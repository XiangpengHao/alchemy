# Alchemy
[![alchemy](https://github.com/XiangpengHao/alchemy/actions/workflows/rust.yml/badge.svg)](https://github.com/XiangpengHao/alchemy/actions/workflows/rust.yml)
[![codecov](https://codecov.io/gh/XiangpengHao/clock-cache/branch/master/graph/badge.svg?token=YS06KZ1RYS)](https://codecov.io/gh/XiangpengHao/clock-cache)

**A**ttribute-**L**evel **C**aching in **He**terogeneous In-**M**emor**y** DBMS

Alchemy is a DRAM-PM hybrid database engine built from scratch to achieve high performance at a low cost. The key is to leverages PM’s byte-addressability to cache only hot attributes in DRAM, reducing requirements on PM bandwidth. 
Alchemy also uses Rust's coroutines to overlap data fetching and computation, effectively hiding CPU stalls. 

### What is PM
PM is a new type of persistent storage technology that is byteaddressable and offers high performance. 
There are multiple alternatives, but [Intel Optane DCPMM](https://www.intel.com/content/www/us/en/architecture-and-technology/optane-dc-persistent-memory.html) built on 3D XPoint is
currently the only commercially available option, which is therefore our focus. 

Compared to DRAM, Optane DCPMM scales to much higher capacity and has a lower price. 
For instance, a single server can be equipped with up to 12TB of Optane DCPMMs, making DCPMM appealing for building persistent and cost-effective memory-optimized OLTP engines. 
However, PM has a relatively smaller bandwidth and a higher access latency compared to DRAM.

### Run Alchemy

Alchemy is built in Rust, and requires nightly Rust to build, you can download here: https://rustup.rs

Alchemy currently only supports Linux server with Intel Optane Persistent Memory Module, it must be configured as AppDirect mode.

[Optional] For more advanced internal metrics, the kernel should enable `msr` module and a [`pcm-sensor-server`](https://github.com/opcm/pcm) should be running in the background

#### Build
```bash
cargo build [--release]
```

#### Test

Run unit test
```bash
cargo test
```

To test with [address sanitizers](https://clang.llvm.org/docs/AddressSanitizer.html):
```bash
env ASAN_OPTIONS="detect_odr_violation=0" RUSTFLAGS="-Z sanitizer=address" cargo test -Zbuild-std --target x86_64-unknown-linux-gnu --features asans -- --skip test_alloc_oom
```


#### Benchmark
Alchemy implements YCSB-like microbenchmarks and TPC-C benchmark.

Run microbenchmark
```bash
POOL_DIR=/mnt/pmem LOG_DIR=/mnt/ssd cargo bench --bench table 
```
The `POOL_DIR` specifies the persistent memory pool directory, it must point to a ext4 filesystem with dax mode enabled.

The `LOG_DIR` specifies the logging directory; the underlying disk must have sufficient write bandwidth (>1GB/s) otherwise logging will be the bottleneck.

Run TPC-C benchmark
```bash
POOL_DIR=/mnt/pmem LOG_DIR=/mnt/ssd cargo bench --bench tpcc 
```

Alchemy uses [Shumai](https://github.com/XiangpengHao/shumai) benchmark framework which provides several convenient features:
it captures the `flamegraph` of benchmark function;
it records the disk bandwidth of the benchmark process;
it collects the `PCM` counter stats.

All benchmark results (along with the statistics) will be saved to a json file.

## Alchemy benchmark results
While we have tried our best, we have not confirmed that we tested everything correctly.
We are happy to work with the community to validate the results and improve the implementation.

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

We open source all of our raw benchmark data in the [`experiments/`](https://github.com/XiangpengHao/alchemy/tree/main/experiments) folder.


## Related projects
[Congee](https://github.com/XiangpengHao/congee): A Rust implementation of concurrent ART (adaptive radix tree). Alchemy uses congee to implement range query.

[Shumai](https://github.com/XiangpengHao/shumai): Shumai is a multi-thread benchmarking framework that produces accurate and reproducible results.
All the benchmarks of Alchemy are powered by Shumai.