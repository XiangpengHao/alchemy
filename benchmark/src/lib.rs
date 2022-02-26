#![feature(const_size_of_val)]
#![feature(const_ptr_offset_from)]
#![feature(iter_advance_by)]

use serde::{Deserialize, Serialize};
use std::fmt;

pub mod micro;
pub mod tpcc;
pub mod utils;

#[macro_export]
macro_rules! mk_field {
    ($root:ident, $parent:tt) => {
        alchemy::query::Field::from_span(memoffset::span_of!($root, $parent))
    };
}

#[macro_export]
macro_rules! mk_query {
    ($root:ident, $($parent:tt),+) => {
        alchemy::query::FieldsMeta::new([
            $(mk_field!($root, $parent),)+
        ])
    };
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CachePolicy {
    Field,
    Tuple,
    Direct,
}

impl fmt::Display for CachePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            CachePolicy::Field => write!(f, "field"),
            CachePolicy::Tuple => write!(f, "tuple"),
            CachePolicy::Direct => write!(f, "direct"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_a() {
        println!("it works");
    }
}
