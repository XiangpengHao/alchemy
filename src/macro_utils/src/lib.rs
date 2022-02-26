extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;

mod tuple;

use proc_macro::TokenStream;

#[proc_macro_derive(Tuple)]
pub fn derive_tuple(input: TokenStream) -> TokenStream {
    tuple::derive_tuple(input)
}
