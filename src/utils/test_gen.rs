use crate::{
    cache_manager::Tuple,
    query::{FieldsMeta, FieldsValue, UpdateQuery},
};
use crate::{
    cache_manager::{QueryValue, Schema},
    query::Field,
};

pub trait TestGen {
    fn from_fixed(seed: usize) -> Self;
    fn from_increasing(seed: usize) -> Self;
}

#[derive(Debug, Clone, PartialEq)]
#[repr(C)]
pub struct TestItem<const TZ: usize> {
    data: [usize; TZ],
}

impl<const TZ: usize> TestItem<TZ> {
    fn offset_mut(&mut self, offset: usize) -> *mut u8 {
        let ptr = &mut self.data as *mut [usize; TZ] as *mut u8;
        unsafe { ptr.add(offset) }
    }

    fn offset(&self, offset: usize) -> *const u8 {
        let ptr = &self.data as *const usize as *const u8;
        unsafe { ptr.add(offset) }
    }

    pub fn build_query<const N: usize>(fields: [u16; N]) -> FieldsMeta<N> {
        assert!(N < TZ);

        let mut rv_fields: [Field; N] = unsafe { std::mem::zeroed() };
        for (i, v) in fields.iter().enumerate() {
            let start = *v * 8;
            rv_fields[i] = Field::new(start, start + 8);
        }
        FieldsMeta::new(rv_fields)
    }

    /// # Safety
    /// Read a field from TestItem can be dangerous if the field len is not properly in range
    pub unsafe fn read_field<T>(&self, field: &Field) -> T {
        let offset = self.offset(field.begin() as usize);
        (offset as *const T).read()
    }
}

impl<const TZ: usize> From<TestItem<TZ>> for usize {
    fn from(v: TestItem<TZ>) -> Self {
        v.data[0]
    }
}

impl<const TZ: usize> TestGen for TestItem<TZ> {
    fn from_fixed(val: usize) -> Self {
        Self { data: [val; TZ] }
    }

    fn from_increasing(base: usize) -> Self {
        let mut data: [usize; TZ] = [0; TZ];
        for (i, item) in data.iter_mut().enumerate() {
            *item = base + i;
        }

        Self { data }
    }
}

impl<const TZ: usize> Tuple for TestItem<TZ> {
    fn update<const N: usize>(&mut self, query: UpdateQuery<'_, N>) {
        for (idx, val) in query.iter() {
            debug_assert_eq!(idx.size() as usize, std::mem::size_of::<usize>());
            let ptr = self.offset_mut(idx.begin() as usize) as *mut usize;
            unsafe {
                ptr.write(*(val as *const usize));
            }
        }
    }

    fn offset_mut(&mut self, offset: usize) -> *mut u8 {
        let ptr = &mut self.data as *mut [usize; TZ] as *mut u8;
        unsafe { ptr.add(offset) }
    }

    fn offset(&self, offset: usize) -> *const u8 {
        let ptr = &self.data as *const usize as *const u8;
        unsafe { ptr.add(offset) }
    }

    fn to_query_values<const N: usize>(&self, query: &FieldsMeta<N>) -> FieldsValue<N> {
        let mut values = [std::ptr::null(); N];
        for (i, fid) in query.iter().enumerate() {
            values[i] = self.offset(fid.begin() as usize) as *const usize as *const u8;
        }
        FieldsValue::new(values)
    }
}

pub struct FieldItemSchema<const FZ: usize, const TZ: usize> {
    fields: [Field; FZ],
}

impl<const FZ: usize, const TZ: usize> FieldItemSchema<FZ, TZ> {
    pub const fn from_fields(fields: [Field; FZ]) -> Self {
        Self { fields }
    }

    pub fn from_range(ranges: [std::ops::Range<u16>; FZ]) -> Self {
        let mut fields: [Field; FZ] = unsafe { std::mem::zeroed() };
        for (i, r) in ranges.iter().enumerate() {
            fields[i] = Field::new(r.start, r.end);
        }
        Self { fields }
    }

    pub fn position(&self, field: &Field) -> Option<usize> {
        self.fields.iter().position(|f| f == field)
    }
}

impl<const FZ: usize, const TZ: usize> Schema for FieldItemSchema<FZ, TZ> {
    type Field = FieldCached<FZ>;
    type Tuple = TestItem<TZ>;

    fn to_cached(&self, tuple: &Self::Tuple) -> Self::Field {
        let mut fields: [usize; FZ] = [0; FZ];
        for (i, v) in self.fields.iter().enumerate() {
            let tuple_data = (&tuple.data) as *const usize;
            fields[i] = unsafe { *tuple_data.add(v.begin() as usize / 8) };
        }
        Self::Field { data: fields }
    }

    fn key(&self, tuple: &Self::Tuple) -> usize {
        tuple.data[0]
    }

    fn write_back(&self, field: &Self::Field, tuple: &mut Self::Tuple) {
        for (c_idx, f) in self.fields.iter().enumerate() {
            let tuple_data = tuple.offset_mut(f.begin() as usize) as *mut usize;
            unsafe {
                tuple_data.write(field.data[c_idx]);
            }
        }
    }

    fn update_cached<'a, const N: usize>(
        &self,
        fields: &mut Self::Field,
        query: UpdateQuery<'a, N>,
    ) {
        for (i, v) in query.iter() {
            if let Some(idx) = self.position(i) {
                debug_assert_eq!(i.size() as usize, std::mem::size_of::<usize>());
                fields.data[idx] = unsafe { *(v as *const usize) };
            }
        }
    }

    fn matches<const Q: usize>(&self, query: &FieldsMeta<Q>) -> bool {
        for f in query.iter() {
            if self.position(f).is_none() {
                return false;
            }
        }
        true
    }

    fn to_cached_field(&self, tuple_field: &Field) -> Option<Field> {
        self.position(tuple_field)
            .map(|i| Field::from_span((i * 8)..(i + 1) * 8))
    }
}

#[derive(Debug, PartialEq)]
pub struct FieldCached<const N: usize> {
    data: [usize; N],
}

#[allow(dead_code)]
#[must_use]
pub(crate) fn value_compare<T, S: Schema, const QN: usize, const TN: usize>(
    values: &QueryValue<<S as Schema>::Field, <S as Schema>::Tuple, T>,
    query: &FieldsMeta<QN>,
    tuple: &TestItem<TN>,
    schema: &S,
) -> bool {
    for q in query.iter() {
        let gt_val: usize = unsafe { tuple.read_field(q) };
        let qy_val = unsafe { values.get_value::<usize, S>(q, schema) };
        if gt_val != *qy_val {
            return false;
        }
    }
    true
}
