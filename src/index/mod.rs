use crate::cache_manager::Rid;
pub use con_art_rust;
use con_art_rust::{Key, UsizeKey};
pub use flurry;

pub trait DbIndex: Send + Sync {
    type Guard;

    fn pin(&self) -> Self::Guard;

    fn get<'a>(&'a self, key: &usize, guard: &'a Self::Guard) -> Option<Rid>;

    fn insert(&self, key: usize, value: Rid, guard: &Self::Guard);

    fn new() -> Self;

    fn with_capacity(cap: usize) -> Self;
}

pub struct Art(pub con_art_rust::Tree<UsizeKey>);

impl Art {
    pub fn range(&self, low: usize, high: usize, out_buffer: &mut [usize]) -> Option<usize> {
        self.0.look_up_range(
            &UsizeKey::key_from(low),
            &UsizeKey::key_from(high),
            out_buffer,
        )
    }
}

impl DbIndex for Art {
    type Guard = con_art_rust::Epoch::Guard;

    fn pin(&self) -> Self::Guard {
        con_art_rust::Epoch::pin()
    }

    fn insert(&self, key: usize, value: Rid, guard: &Self::Guard) {
        self.0
            .insert(UsizeKey::key_from(key), value.as_u32() as usize, guard);
    }

    fn get<'a>(&'a self, key: &usize, guard: &'a Self::Guard) -> Option<Rid> {
        let usize_v = self.0.get(&UsizeKey::key_from(*key), guard)?;
        Some(Rid::from_u32(usize_v as u32))
    }

    fn new() -> Self {
        Self(con_art_rust::Tree::new())
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
