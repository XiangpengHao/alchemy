#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

pub(crate) struct NodeMeta {
    data: AtomicU32,
}

impl NodeMeta {
    const HOTNESS_MASK: u32 = 0xff00_0000; // the first byte
    const HOTNESS_SHIFT: usize = 24;
    const DIRTY_MASK: u32 = 0x0010_0000;
    const REF_MASK: u32 = 0x0001_0000;

    pub fn new() -> Self {
        Self {
            data: AtomicU32::new(0),
        }
    }

    pub fn hotness(&self) -> usize {
        (self.data.load(Ordering::Relaxed) >> Self::HOTNESS_SHIFT) as usize
    }

    #[allow(dead_code)]
    pub fn is_dirty(&self) -> bool {
        self.data.load(Ordering::Relaxed) & Self::DIRTY_MASK > 0
    }

    pub fn set_dirty(&self) {
        let _old = self.data.fetch_or(Self::DIRTY_MASK, Ordering::Relaxed);
    }

    pub fn set_ref(&self) {
        self.data.fetch_or(Self::REF_MASK, Ordering::Relaxed);
    }

    pub fn clear_ref(&self) {
        self.data.fetch_and(!Self::REF_MASK, Ordering::Relaxed);
    }

    // Clear the ref bit, return true if the ref bit is set
    pub fn clear_if_set(&self) -> bool {
        let data = self.data.load(Ordering::Relaxed);
        if (data & Self::REF_MASK) > 0 {
            self.data.store(data & (!Self::REF_MASK), Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    pub fn is_refed(&self) -> bool {
        let data = self.data.load(Ordering::Relaxed);
        (data & Self::REF_MASK) > 0
    }

    /// `inc_hotness` has a minor issue:
    /// When hotness reached more than u8::MAX, it becomes 0 because integer overflow
    /// This lead to bad hotness approximation but given the fact that hotness is not that important,
    /// we are ok with this.
    /// We stick to this because fetch_add is known to be much faster than compare_and_swap
    /// Although the impact here is unknown
    pub fn inc_hotness(&self, amount: u32) -> u32 {
        let mut old = self
            .data
            .fetch_add((amount as u32) << Self::HOTNESS_SHIFT, Ordering::Relaxed);
        old >>= Self::HOTNESS_SHIFT;
        old
    }

    pub fn try_dec_hotness(&self, amount: u32) -> bool {
        let amount_shifted = amount << Self::HOTNESS_SHIFT;
        let old = self.data.load(Ordering::Relaxed);
        let mut hotness = old;
        if hotness >= amount_shifted {
            hotness -= amount_shifted;
        } else {
            return false;
        }
        let new = (old & !Self::HOTNESS_MASK) | hotness;
        let _rv = self
            .data
            .compare_exchange_weak(old, new, Ordering::Relaxed, Ordering::Relaxed);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meta_dirty() {
        let meta = NodeMeta::new();

        assert!(!meta.is_dirty());

        meta.set_dirty();

        assert!(meta.is_dirty());
    }

    #[test]
    fn meta_hotness() {
        let meta = NodeMeta::new();

        let mut hotness = meta.hotness();
        assert_eq!(hotness, 0);

        meta.inc_hotness(1);
        meta.inc_hotness(1);
        hotness = meta.hotness();
        assert_eq!(hotness, 2);

        let success = meta.try_dec_hotness(1);
        assert_eq!(success, true);
        hotness = meta.hotness();
        assert_eq!(hotness, 1);
    }

    use crossbeam_utils::thread::scope;
    use rand::Rng;

    #[test]
    fn multi_hotness() {
        let meta = NodeMeta::new();

        let threads = 4;
        let operation = 1024 * 16;

        scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|_| {
                    let mut rng = rand::thread_rng();
                    for _ in 0..operation {
                        let op = rng.gen_bool(0.5);
                        match op {
                            true => {
                                meta.inc_hotness(1);
                            }
                            false => {
                                meta.try_dec_hotness(1);
                            }
                        };
                        assert!(!meta.is_dirty());
                    }
                });
            }
        })
        .unwrap();

        meta.set_dirty();

        scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|_| {
                    let mut rng = rand::thread_rng();
                    for _ in 0..operation {
                        let op = rng.gen_bool(0.5);
                        match op {
                            true => {
                                meta.inc_hotness(1);
                            }
                            false => {
                                meta.try_dec_hotness(1);
                            }
                        };

                        assert!(meta.is_dirty());
                    }
                });
            }
        })
        .unwrap();
    }
}
