// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Derive macros for [clapsmith](https://docs.rs/clapsmith). Use them through
//! that crate (`clapsmith::Dispatch`), not directly.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input, spanned::Spanned};

/// Implements `clapsmith::Handler` for an enum of commands by passing each
/// variant's value to its own handler.
///
/// Every variant must hold exactly one field, and that field's type must
/// implement `Handler` for the same context. A variant without a handler is a
/// compile error.
#[proc_macro_derive(Dispatch)]
pub fn derive_dispatch(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "Dispatch can only be derived for an enum of commands",
        ));
    };
    let name = &input.ident;
    let mut arms = Vec::new();
    let mut bounds = Vec::new();
    for variant in &data.variants {
        let ident = &variant.ident;
        let field = match &variant.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => &fields.unnamed[0],
            _ => {
                return Err(syn::Error::new(
                    variant.span(),
                    "each Dispatch variant must hold exactly one value, like `Run(RunCmd)`",
                ));
            }
        };
        let ty = &field.ty;
        arms.push(quote! { Self::#ident(command) => ::clapsmith::Handler::run(command, ctx), });
        bounds.push(quote! { #ty: ::clapsmith::Handler<__Ctx>, });
    }
    let (_, ty_generics, _) = input.generics.split_for_impl();
    let params = &input.generics.params;
    let where_clause = input.generics.where_clause.as_ref().map(|w| &w.predicates);
    Ok(quote! {
        impl<__Ctx, #params> ::clapsmith::Handler<__Ctx> for #name #ty_generics
        where
            #(#bounds)*
            #where_clause
        {
            fn run(self, ctx: &mut __Ctx) -> ::clapsmith::Result {
                match self {
                    #(#arms)*
                }
            }
        }
    })
}
