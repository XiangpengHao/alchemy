pub mod clock_cache;

use std::{cell::UnsafeCell, marker::PhantomData};

use crate::{
    async_task::{DummySwitcher, PrefetchRef},
    query::{Field, FieldsMeta, FieldsValue, UpdateQuery},
    utils::prefetch,
};

pub(crate) use clock_cache::ClockCache;

use self::clock_cache::oid::OidWriteGuard;

/// RID: record id, is the offset to the table storage
#[derive(Debug, Copy, Clone)]
pub struct Rid {
    id: u32,
}

impl Rid {
    #[inline]
    pub fn as_u32(&self) -> u32 {
        self.id
    }

    pub fn from_u32(id: u32) -> Self {
        debug_assert!(
            id != u32::MAX,
            "id is u32::MAX which is reserved for sentinel"
        );
        Self { id }
    }
}

impl From<usize> for Rid {
    #[inline]
    fn from(id: usize) -> Self {
        Rid::from_u32(id as u32)
    }
}

impl From<Rid> for usize {
    #[inline]
    fn from(rid: Rid) -> Self {
        rid.id as usize
    }
}

pub struct QueryValue<'a, F, T: Tuple, L> {
    field: Option<&'a UnsafeCell<F>>,
    tuple: Option<&'a UnsafeCell<T>>,
    pt_l: PhantomData<L>,
}

impl<'a, F, T: Tuple> QueryValue<'a, F, T, OidWriteGuard<'a>> {
    /// Get a mutable reference to the field
    /// # Safety
    /// The caller need to ensure that `field` is a proper field, e.g. at least within the range of tuple
    #[allow(clippy::unnecessary_unwrap)]
    pub unsafe fn get_value_mut<V>(&self, field: &Field, schema: &impl Schema) -> &'a mut V {
        let cached_field = schema.to_cached_field(field);

        if cached_field.is_some() && self.field.is_some() {
            let cached = self.field.unwrap().get() as *mut u8;
            &mut *(cached.add(cached_field.unwrap().begin() as usize) as *mut V)
        } else {
            let tuple = self
                .tuple
                .expect("cached item can't satisfy the queried item")
                .get() as *mut u8;
            &mut *(tuple.add(field.begin() as usize) as *mut V)
        }
    }
}

impl<'a, F, T: Tuple, L> QueryValue<'a, F, T, L> {
    pub fn new(field: Option<&'a UnsafeCell<F>>, tuple: Option<&'a UnsafeCell<T>>) -> Self {
        Self {
            field,
            tuple,
            pt_l: PhantomData,
        }
    }

    /// Get the value reference from query
    /// # Safety
    /// The caller need to ensure that `field` is a proper field, e.g. at least within the range of tuple
    #[allow(clippy::unnecessary_unwrap)]
    pub unsafe fn get_value<V, S: Schema>(&self, field: &Field, schema: &S) -> PrefetchRef<V> {
        let cached_field = schema.to_cached_field(field);

        if cached_field.is_some() && self.field.is_some() {
            let cached = self.field.unwrap().get() as *const u8;
            let prefetched = &*(cached.add(cached_field.unwrap().begin() as usize) as *const V);
            PrefetchRef::new(prefetched)
        } else {
            let tuple = self
                .tuple
                .expect("cached item can't satisfy the queried item")
                .get() as *const u8;
            let prefetched = &*(tuple.add(field.begin() as usize) as *const V);
            PrefetchRef::new(prefetched)
        }
    }

    pub async fn prefetch_all(&self) {
        if let Some(field) = self.field {
            prefetch::mem_prefetch_n(field.get() as *const i8, std::mem::size_of::<F>());
        }
        if let Some(tuple) = self.tuple {
            prefetch::mem_prefetch_n(tuple.get() as *const i8, std::mem::size_of::<T>());
            DummySwitcher::new(1).await
        }
    }

    pub fn cached_hit(&self) -> bool {
        self.field.is_some()
    }

    pub fn has_field(&self) -> bool {
        self.field.is_some()
    }

    pub fn has_tuple(&self) -> bool {
        self.tuple.is_some()
    }
}

pub trait Updatable {
    fn update<const N: usize>(&mut self, query: UpdateQuery<'_, N>);
}

pub trait Tuple: Clone {
    fn update<const N: usize>(&mut self, query: UpdateQuery<'_, N>);
    fn offset(&self, offset: usize) -> *const u8;
    fn offset_mut(&mut self, offset: usize) -> *mut u8;
    fn to_query_values<const N: usize>(&self, query: &FieldsMeta<N>) -> FieldsValue<N>;
}

pub trait Schema: Send + Sync {
    type Field;
    type Tuple: Tuple + Clone + 'static;

    /// make a field out from the full tuple.
    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field;

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple);

    fn update_cached<'a, const N: usize>(
        &self,
        field: &mut Self::Field,
        query: UpdateQuery<'a, N>,
    ) {
        for (i, v) in query.iter() {
            let field_ptr = field as *mut Self::Field as *mut u8;
            if let Some(f) = self.to_cached_field(i) {
                unsafe {
                    std::ptr::copy_nonoverlapping(v, field_ptr.add(f.begin() as usize), f.size());
                }
            }
        }
    }

    /// Generate unique primary key form the tuple
    fn key(&self, tuple: &Self::Tuple) -> usize;

    /// check if the schema matches with the query
    fn matches<const N: usize>(&self, query: &FieldsMeta<N>) -> bool {
        for f in query.iter() {
            if self.to_cached_field(f).is_none() {
                return false;
            }
        }
        true
    }

    /// Convert a field in tuple to a field in cached field
    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field>;
}
