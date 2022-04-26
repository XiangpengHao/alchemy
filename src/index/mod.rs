use crate::attribute_cache::Rid;
pub use congee;
use congee::epoch::Guard;
pub use flurry;

pub trait DbIndex: Send + Sync {
    type Guard;

    fn pin(&self) -> Self::Guard;

    fn get<'a>(&'a self, key: &usize, guard: &'a Self::Guard) -> Option<Rid>;

    fn insert(&self, key: usize, value: Rid, guard: &Self::Guard);

    fn new() -> Self;

    fn with_capacity(cap: usize) -> Self;
}

pub struct Art(pub congee::ArtRaw<usize, Rid>);

impl Art {
    pub fn range(
        &self,
        low: usize,
        high: usize,
        out_buffer: &mut [(usize, usize)],
        guard: &Guard,
    ) -> usize {
        self.0.range(&low, &high, out_buffer, guard)
    }
}

impl DbIndex for Art {
    type Guard = congee::epoch::Guard;

    #[inline]
    fn pin(&self) -> Self::Guard {
        congee::epoch::pin()
    }

    #[inline]
    fn insert(&self, key: usize, value: Rid, guard: &Self::Guard) {
        self.0.insert(key, value, guard);
    }

    #[inline]
    fn get<'a>(&'a self, key: &usize, guard: &'a Self::Guard) -> Option<Rid> {
        let v = self.0.get(key, guard)?;
        Some(v)
    }

    fn new() -> Self {
        Self(congee::ArtRaw::new())
    }

    fn with_capacity(_cap: usize) -> Self {
        Self::new()
    }
}

pub struct FlurryMap(flurry::HashMap<usize, Rid>);

impl DbIndex for FlurryMap {
    type Guard = flurry::epoch::Guard;

    fn pin(&self) -> Self::Guard {
        flurry::epoch::pin()
    }

    fn get<'a>(&'a self, key: &usize, guard: &'a Self::Guard) -> Option<Rid> {
        let rid = self.0.get(key, guard)?;
        Some(*rid)
    }

    fn insert(&self, key: usize, value: Rid, guard: &Self::Guard) {
        self.0.insert(key, value, guard);
    }

    fn new() -> Self {
        FlurryMap(flurry::HashMap::<usize, Rid>::new())
    }

    fn with_capacity(cap: usize) -> Self {
        FlurryMap(flurry::HashMap::<usize, Rid>::with_capacity(cap))
    }
}
