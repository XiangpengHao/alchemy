use std::{marker::PhantomData, u16};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Field {
    begin: u16,
    end: u16,
}

impl Field {
    pub const fn default() -> Self {
        Self { begin: 0, end: 0 }
    }

    pub const fn new(begin: u16, end: u16) -> Self {
        Self { begin, end }
    }

    pub const fn from_span(span: std::ops::Range<usize>) -> Self {
        Self {
            begin: span.start as u16,
            end: span.end as u16,
        }
    }

    pub const fn begin(&self) -> u16 {
        self.begin
    }

    pub const fn end(&self) -> u16 {
        self.end
    }

    pub const fn size(&self) -> usize {
        (self.end - self.begin) as usize
    }
}

/// The read query format for our database system
/// The query need to specify the field id and field length
#[derive(Debug, Clone)]
pub struct FieldsMeta<const N: usize> {
    pub fields: [Field; N],
}

impl<const N: usize> FieldsMeta<N> {
    pub fn iter(&self) -> std::slice::Iter<'_, Field> {
        self.fields.iter()
    }

    pub const fn new(fields: [Field; N]) -> Self {
        Self { fields }
    }

    pub fn from_spans(spans: [std::ops::Range<usize>; N]) -> Self {
        let mut fields = [Field::default(); N];
        for (idx, f) in fields.iter_mut().enumerate() {
            *f = Field::from_span(spans[idx].clone());
        }
        Self { fields }
    }

    pub fn size(&self) -> usize {
        self.fields.iter().fold(0, |a, b| a + b.size())
    }
}

/// The update query,
/// The `meta` describe the fields it needs to update,
/// the data holds the actual data
#[derive(Debug, Clone)]
pub struct UpdateQuery<'a, const N: usize> {
    meta: FieldsMeta<N>,
    data: &'a [u8],
}

unsafe impl<const N: usize> Send for UpdateQuery<'_, N> {}
unsafe impl<const N: usize> Sync for UpdateQuery<'_, N> {}

impl<'a, const N: usize> UpdateQuery<'a, N> {
    pub fn new(fields: [Field; N], data: &'a [u8]) -> Self {
        let meta = FieldsMeta::new(fields);
        Self::from_meta(meta, data)
    }

    pub fn from_meta(meta: FieldsMeta<N>, data: &'a [u8]) -> Self {
        Self { meta, data }
    }

    pub const fn iter(&self) -> UpdateIter<N> {
        UpdateIter {
            cur_idx: 0,
            ptr_off: 0,
            query: self,
        }
    }

    pub const fn meta(&self) -> &FieldsMeta<N> {
        &self.meta
    }
}

pub struct UpdateIter<'a, const N: usize> {
    cur_idx: usize,
    ptr_off: usize,
    query: &'a UpdateQuery<'a, N>,
}

impl<'a, const N: usize> Iterator for UpdateIter<'a, N> {
    type Item = (&'a Field, *const u8);

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_idx == N {
            None
        } else {
            let idx = self.cur_idx;
            let ptr_off = self.ptr_off;
            self.cur_idx += 1;

            unsafe {
                let field = self.query.meta.fields.get_unchecked(idx);
                self.ptr_off += field.size() as usize;
                let cur_ptr = self.query.data.as_ptr().add(ptr_off);
                Some((field, cur_ptr))
            }
        }
    }
}

/// FieldValue is the response to `FieldQuery`,
/// where each `*mut u8` points to a field it queried
#[derive(Debug, Clone)]
pub struct FieldsValue<'a, const N: usize> {
    data: [*const u8; N],
    lt_pt: PhantomData<&'a ()>,
}

impl<const N: usize> FieldsValue<'_, N> {
    pub fn new(values: [*const u8; N]) -> Self {
        Self {
            data: values,
            lt_pt: PhantomData,
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, *const u8> {
        self.data.iter()
    }

    pub fn get(&self, idx: usize) -> *const u8 {
        debug_assert!(idx < N);
        unsafe { *self.data.get_unchecked(idx) }
    }
}
