use alchemy::error::TransactionError;
use metric::Counter;
use once_cell::sync::OnceCell;
use rand::{distributions::Alphanumeric, prelude::SmallRng, thread_rng, Rng, SeedableRng};
use std::str::FromStr;

#[allow(dead_code)]
#[derive(Debug, Clone, Hash)]
pub struct FixedString<const N: usize> {
    data: [u8; N],
}

impl<const N: usize> Default for FixedString<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> PartialEq<&str> for FixedString<N> {
    fn eq(&self, other: &&str) -> bool {
        if other.len() != N {
            return false;
        }
        for (i, v) in self.data.iter().enumerate() {
            if *v != other.as_bytes()[i] {
                return false;
            }
        }
        true
    }
}

impl<const N: usize> FromStr for FixedString<N> {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        let mut data = [0; N];
        for (idx, i) in data.iter_mut().enumerate() {
            *i = s.as_bytes()[idx];
        }
        Ok(Self { data })
    }
}

impl<const N: usize> FixedString<N> {
    pub fn rand_gen() -> Self {
        let mut data = [0; N];
        let mut rng = thread_rng();
        for i in data.iter_mut() {
            *i = rng.sample(Alphanumeric);
        }
        Self { data }
    }

    pub fn copy_from(&mut self, src: &str, start: usize) {
        debug_assert!(start + src.len() < N);
        for (idx, i) in src.as_bytes().iter().enumerate() {
            self.data[idx] = *i;
        }
    }

    pub fn new() -> Self {
        Self { data: [0; N] }
    }

    pub fn concat<const F: usize, const S: usize>(
        first: &FixedString<F>,
        second: &FixedString<S>,
        space: usize,
    ) -> FixedString<N> {
        assert!(F + S + space <= N);
        let mut data = [0; N];
        unsafe {
            std::ptr::copy_nonoverlapping(
                &first.data as *const u8,
                &mut data as *mut [u8; N] as *mut u8,
                F,
            );
            std::ptr::copy_nonoverlapping(
                &second.data as *const u8,
                (&mut data as *mut [u8; N] as *mut u8).add(F + space),
                S,
            )
        }

        FixedString { data }
    }
}

/// uniform random number generator, as per TPCC 2.1.5
pub fn u_rand(x: u64, y: u64) -> u64 {
    let mut rng = thread_rng();
    rng.gen_range(x..=y)
}

/// None-uniform random number generator, as per TPCC 2.1.6
pub fn nu_rand(a: u64, x: u64, y: u64) -> u64 {
    let rng = thread_rng();
    let mut rng = SmallRng::from_rng(rng).unwrap();

    static C_255: OnceCell<u64> = OnceCell::new();
    static C_1023: OnceCell<u64> = OnceCell::new();
    static C_8191: OnceCell<u64> = OnceCell::new();

    let c = match a {
        255 => C_255.get_or_init(|| rng.gen_range(0..=a)),
        1023 => C_1023.get_or_init(|| rng.gen_range(0..=a)),
        8191 => C_8191.get_or_init(|| rng.gen_range(0..=a)),
        _ => panic!(
            "invalid a, a must be one of 255, 1023 or 8191, got a = {}",
            a
        ),
    };

    ((rng.gen_range(0..=a) | rng.gen_range(x..=y)) + *c) % (y - x + 1) + x
}

/// Align the given `size` upwards to alignment `align`.
///
/// Requires that `align` is a power of two.
pub(crate) fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) & !(align - 1)
}

pub(crate) fn zip_gen() -> u64 {
    (thread_rng().gen_range(0..=9999) << 10) + 1111
}

/// Generate a last name in customer table as per TPCC 4.3.3.1
/// The seed must be a three-digit number
pub(crate) fn last_name_gen(mut seed: u32) -> FixedString<16> {
    let name_bank = [
        "BAR", "OUGHT", "ABLE", "PRI", "PRES", "ESE", "ANTI", "CALLY", "ATION", "EING",
    ];

    let mut last_name = FixedString::new();
    let mut pos = 0;
    for _i in 0..3 {
        let idx = seed % 10;
        seed /= 10;
        last_name.copy_from(name_bank[idx as usize], pos);
        pos += name_bank[idx as usize].len();
    }
    last_name
}

pub fn txn_err_to_counter(e: &TransactionError) -> Counter {
    match e {
        TransactionError::IndexNotFound => {
            panic!(
                "Index not found is not possible with TPCC transactions, something must be wrong!"
            )
        }
        TransactionError::LockBusy => Counter::AbortLockBusy,
        TransactionError::ReadLockBusy => Counter::AbortRLockBusy,
        TransactionError::WriteLockBusy => Counter::AbortWLockBusy,
        TransactionError::UpgradeLockBusy => Counter::AbortUpgradeBusy,
        TransactionError::UserAbort => Counter::NewOrderRollback,
    }
}
#[cfg(test)]
mod tests {

    #[test]
    fn nu_rand_1023() {
        for _ in 0..1000 {
            println!("{}\t", super::nu_rand(8191, 0, 999));
        }
    }
}
