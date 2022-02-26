mod txn_result;
use std::ops::Range;

use bincode::Encode;
use rand::{prelude::ThreadRng, Rng};
use rand_distr::{Distribution, Uniform};
use selfsimilar::SelfSimilarDistribution;
use zipf::ZipfDistribution;

use self::config::Workload;

pub use txn_result::TransactionResult;

pub mod config;

pub enum RandDistribution {
    Zipfian(ZipfDistribution),
    Uniform(Uniform<usize>),
    SelfSimilar(SelfSimilarDistribution),
}

impl RandDistribution {
    pub fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> usize {
        match self {
            Self::Zipfian(z) => z.sample(rng),
            Self::SelfSimilar(s) => s.sample(rng),
            Self::Uniform(u) => u.sample(rng),
        }
    }

    pub fn from(dist: config::Distribution, item_range: Range<usize>) -> Self {
        match dist {
            config::Distribution::SelfSimilar05 => RandDistribution::SelfSimilar(
                SelfSimilarDistribution::new(item_range.start, item_range.end, 0.5),
            ),
            config::Distribution::SelfSimilar04 => RandDistribution::SelfSimilar(
                SelfSimilarDistribution::new(item_range.start, item_range.end, 0.4),
            ),
            config::Distribution::SelfSimilar03 => RandDistribution::SelfSimilar(
                SelfSimilarDistribution::new(item_range.start, item_range.end, 0.3),
            ),
            config::Distribution::SelfSimilar02 => RandDistribution::SelfSimilar(
                SelfSimilarDistribution::new(item_range.start, item_range.end, 0.2),
            ),
            config::Distribution::SelfSimilar01 => RandDistribution::SelfSimilar(
                SelfSimilarDistribution::new(item_range.start, item_range.end, 0.1),
            ),
            config::Distribution::Uniform => RandDistribution::Uniform(Uniform::from(item_range)),
            config::Distribution::Zipfian => {
                RandDistribution::Zipfian(ZipfDistribution::new(item_range.end - 1, 1.003).unwrap())
            }
        }
    }
}

pub enum Operation {
    Read,
    Update,
    Insert,
}

impl Operation {
    pub fn gen(rng: &mut ThreadRng, workload: Workload) -> Self {
        match workload {
            config::Workload::ReadOnly => Operation::Read,
            config::Workload::ReadHeavy => {
                if rng.gen_range(0..4) == 0 {
                    Operation::Update
                } else {
                    Operation::Read
                }
            }
            config::Workload::Balanced => {
                if rng.gen_range(0..2) == 0 {
                    Operation::Update
                } else {
                    Operation::Read
                }
            }
            config::Workload::WriteHeavy => {
                if rng.gen_range(0..4) == 0 {
                    Operation::Read
                } else {
                    Operation::Update
                }
            }
            config::Workload::WriteOnly => Operation::Update,
            config::Workload::InsertOnly => Operation::Insert,
        }
    }
}
#[derive(Encode, Clone, Copy)]
pub struct WriteLogEntry<const N: usize> {
    key: usize,
    values: [u32; N],
}

impl<const N: usize> Default for WriteLogEntry<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> WriteLogEntry<N> {
    fn new() -> Self {
        Self {
            key: 0,
            values: [0; N],
        }
    }

    pub fn create_write(key: usize, values: [usize; N]) -> Self {
        let mut compact_values = [0; N];
        for (i, v) in values.iter().enumerate() {
            compact_values[i] = *v as u32;
        }
        WriteLogEntry {
            key: key as usize,
            values: compact_values,
        }
    }
}
