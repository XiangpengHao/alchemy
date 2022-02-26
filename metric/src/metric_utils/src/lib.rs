extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;

use proc_macro::TokenStream;
use syn::Expr;

/// It implements a few helper functions for unit Enums:
/// ```ignore
/// #[derive(MetricEnum)]
/// pub enum Counter {
///     ClockHit = 0,
///     ClockMiss = 1,
///     ProbeMiss = 2,
///     SchemaMiss = 3,
/// }
/// ```
/// Into:
/// ```ignore
/// impl Counter {
///     pub const LENGTH: usize = 4;
///     pub fn from_num(num: usize) -> Self{
///         match num {
///             0 => Self::ClockHit,
///             ...
///             _ => unreachable!(),
///         }
///     }
/// }
/// ```
#[proc_macro_derive(MetricEnum)]
pub fn derive_enum_count(input: TokenStream) -> TokenStream {
    let syn_item: syn::DeriveInput = syn::parse(input).unwrap();
    let enum_name = syn_item.ident;
    let variants = match syn_item.data {
        syn::Data::Enum(enum_item) => enum_item.variants,
        _ => panic!("MetricEnum only works on Enums"),
    };
    let len = variants.len();

    let arms = variants.iter().map(|v| {
        match v.fields {
            syn::Fields::Unit => {}
            _ => panic!("only unit enum is supported!"),
        };
        let name = &v.ident;
        let dis: &Expr = &v.discriminant.as_ref().unwrap().1;
        let dis = match dis {
            Expr::Lit(v) => v,
            _ => panic!("only number literal is allow in discriminant!"),
        };
        let dis = &dis.lit;
        let dis = match dis {
            syn::Lit::Int(v) => v,
            _ => panic!("only int literal is allowed!"),
        };
        let dis = dis.to_string().parse::<u8>().unwrap() as usize;

        quote! {
            #dis => Self::#name,
        }
    });

    let expanded = quote! {
        impl #enum_name {
            pub const LENGTH: usize = #len;
            pub fn from_num(num: usize) -> Self{
                match num {
                    #(#arms)*
                    _ => unreachable!(),
                }
            }
        }
    };
    expanded.into()
}
