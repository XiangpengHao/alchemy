use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

pub fn derive_tuple(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = ast.ident;

    let output = quote! {
        impl alchemy::attribute_cache::Tuple for #name {
            fn to_query_values<const N: usize>(
                &self,
                query: &alchemy::query::FieldsMeta<N>,
            ) -> alchemy::query::FieldsValue<N> {
                let mut values = [std::ptr::null(); N];
                for (i, fid) in query.iter().enumerate() {
                    values[i] = unsafe { (self as *const Self as *const u8).add(fid.begin() as usize) };
                }
                alchemy::query::FieldsValue::new(values)
            }

            fn offset(&self, offset: usize) -> *const u8 {
                debug_assert!(offset < std::mem::size_of::<Self>());
                let ptr = self as *const Self as *const u8;
                unsafe { ptr.add(offset) }
            }

            fn offset_mut(&mut self, offset: usize) -> *mut u8 {
                debug_assert!(offset < std::mem::size_of::<Self>());
                let ptr = self as *mut Self as *mut u8;
                unsafe { ptr.add(offset) }
            }

            fn update<const N: usize>(&mut self, query: alchemy::query::UpdateQuery<'_, N>){
                for (idx, val) in query.iter() {
                    let ptr = self.offset_mut(idx.begin() as usize);
                    unsafe {
                        std::ptr::copy_nonoverlapping(val, ptr, idx.size());
                    }
                }
            }

        }
    };
    output.into()
}
