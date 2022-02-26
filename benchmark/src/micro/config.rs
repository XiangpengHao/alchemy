use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum Distribution {
    Uniform,
    Zipfian,
    SelfSimilar01,
    SelfSimilar02,
    SelfSimilar03,
    SelfSimilar04,
    SelfSimilar05,
}

impl fmt::Display for Distribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Distribution::Uniform => write!(f, "uniform"),
            Distribution::Zipfian => write!(f, "zipfian"),
            Distribution::SelfSimilar01 => write!(f, "selfsimilar01"),
            Distribution::SelfSimilar02 => write!(f, "selfsimilar02"),
            Distribution::SelfSimilar03 => write!(f, "selfsimilar03"),
            Distribution::SelfSimilar04 => write!(f, "selfsimilar04"),
            Distribution::SelfSimilar05 => write!(f, "selfsimilar05"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Workload {
    ReadOnly,
    ReadHeavy,
    Balanced,
    WriteHeavy,
    WriteOnly,
    InsertOnly,
}

impl fmt::Display for Workload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Workload::ReadOnly => write!(f, "read_only"),
            Workload::ReadHeavy => write!(f, "read_heavy"),
            Workload::Balanced => write!(f, "balanced"),
            Workload::WriteHeavy => write!(f, "write_heavy"),
            Workload::WriteOnly => write!(f, "write_only"),
            Workload::InsertOnly => write!(f, "insert_only"),
        }
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum Replacement {
    None,
    Clock,
    Direct,
}

impl fmt::Display for Replacement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Replacement::Clock => write!(f, "clock"),
            Replacement::None => write!(f, "none"),
            Replacement::Direct => write!(f, "direct"),
        }
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum MemEngine {
    Sync,
    Async,
}

impl fmt::Display for MemEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Sync => write!(f, "sync"),
            Self::Async => write!(f, "async"),
        }
    }
}

pub mod bench_config {
    use crate::CachePolicy;
    use shumai::config;

    use super::{Distribution, MemEngine, Replacement, Workload};

    #[config(path = "micro.toml")]
    pub struct Cache {
        pub name: String,
        pub threads: Vec<usize>,
        pub cache_size_mb: usize,
        pub data_size_mb: usize,
        pub field_cnt: usize,
        pub field_cached_cnt: usize,
        #[matrix]
        pub probe_len: usize,
        pub operation_m: usize,
        #[matrix]
        pub distribution: Distribution,
        #[matrix]
        pub replacement: Replacement,
        pub time: usize,
    }

    #[config(path = "micro.toml")]
    pub struct Table {
        pub name: String,
        pub threads: Vec<usize>,
        #[matrix]
        pub cache_size_mb: usize,
        pub data_size_mb: usize,
        #[matrix]
        pub probe_len: usize,
        #[matrix]
        pub distribution: Distribution,
        #[matrix]
        pub policy: CachePolicy,
        #[matrix]
        pub workload: Workload,
        pub time: usize,
        #[matrix]
        pub probe_rng: f32,
        #[matrix]
        pub mem_engine: MemEngine,
    }

    #[config(path = "micro.toml")]
    pub struct Transaction {
        pub name: String,
        pub time: usize,
        pub threads: Vec<usize>,
        #[matrix]
        pub customer_cache_mb: usize,
        pub customer_data_mb: usize,
        pub item_cache_mb: usize,
        pub item_data_mb: usize,
        #[matrix]
        pub mem_engine: MemEngine,
    }

    #[config(path = "micro.toml")]
    pub struct Schema {
        pub name: String,
        pub time: usize,
        pub threads: Vec<usize>,
        #[matrix]
        pub workload: Workload,
        #[matrix]
        pub schema_cnt: usize,
        #[matrix]
        pub mem_engine: MemEngine,
    }
}
