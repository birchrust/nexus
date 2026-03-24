//! Derive macros for nexus-rt.
//!
//! Use `nexus-rt` instead of depending on this crate directly.
//! The derives are re-exported from `nexus_rt::{Resource, Deref, DerefMut}`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

// =============================================================================
// #[derive(Resource)]
// =============================================================================

/// Derive the `Resource` marker trait, allowing this type to be stored
/// in a [`World`](nexus_rt::World).
///
/// ```ignore
/// use nexus_rt::Resource;
///
/// #[derive(Resource)]
/// struct OrderBook {
///     bids: Vec<(f64, f64)>,
///     asks: Vec<(f64, f64)>,
/// }
/// ```
#[proc_macro_derive(Resource)]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics ::nexus_rt::Resource for #name #ty_generics
            #where_clause
        {}
    }
    .into()
}

// =============================================================================
// #[derive(Deref)]
// =============================================================================

/// Derive `Deref` for newtype wrappers.
///
/// - Single-field structs: auto-selects the field.
/// - Multi-field structs: requires `#[deref]` on exactly one field.
///
/// ```ignore
/// use nexus_rt::Deref;
///
/// #[derive(Deref)]
/// struct MyWrapper(u64);
///
/// #[derive(Deref)]
/// struct Named {
///     #[deref]
///     data: Vec<u8>,
///     label: String,
/// }
/// ```
#[proc_macro_derive(Deref, attributes(deref))]
pub fn derive_deref(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let (field_ty, field_access) = match deref_field(&input.data, name) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };

    quote! {
        impl #impl_generics ::core::ops::Deref for #name #ty_generics
            #where_clause
        {
            type Target = #field_ty;

            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.#field_access
            }
        }
    }
    .into()
}

// =============================================================================
// #[derive(DerefMut)]
// =============================================================================

/// Derive `DerefMut` for newtype wrappers.
///
/// Same field selection rules as `#[derive(Deref)]`. Must be used
/// alongside `#[derive(Deref)]`.
#[proc_macro_derive(DerefMut, attributes(deref))]
pub fn derive_deref_mut(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let (_field_ty, field_access) = match deref_field(&input.data, name) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };

    quote! {
        impl #impl_generics ::core::ops::DerefMut for #name #ty_generics
            #where_clause
        {
            #[inline]
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.#field_access
            }
        }
    }
    .into()
}

// =============================================================================
// Shared field resolution
// =============================================================================

/// Find the deref target field. Returns (field_type, field_access).
fn deref_field(
    data: &Data,
    name: &syn::Ident,
) -> Result<(syn::Type, proc_macro2::TokenStream), syn::Error> {
    let fields = match data {
        Data::Struct(s) => &s.fields,
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                name,
                "Deref/DerefMut can only be derived for structs, not enums",
            ));
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                name,
                "Deref/DerefMut can only be derived for structs, not unions",
            ));
        }
    };

    match fields {
        // Tuple struct: single field → auto-select
        Fields::Unnamed(f) if f.unnamed.len() == 1 => {
            let field = f.unnamed.first().unwrap();
            let ty = field.ty.clone();
            let access = quote!(0);
            Ok((ty, access))
        }
        // Named struct: single field → auto-select
        Fields::Named(f) if f.named.len() == 1 => {
            let field = f.named.first().unwrap();
            let ty = field.ty.clone();
            let ident = field.ident.as_ref().unwrap();
            let access = quote!(#ident);
            Ok((ty, access))
        }
        // Multiple fields → look for #[deref] attribute
        Fields::Named(f) => {
            let marked: Vec<_> = f
                .named
                .iter()
                .filter(|field| field.attrs.iter().any(|a| a.path().is_ident("deref")))
                .collect();

            match marked.len() {
                0 => Err(syn::Error::new_spanned(
                    name,
                    "multiple fields require exactly one `#[deref]` attribute",
                )),
                1 => {
                    let field = marked[0];
                    let ty = field.ty.clone();
                    let ident = field.ident.as_ref().unwrap();
                    let access = quote!(#ident);
                    Ok((ty, access))
                }
                _ => Err(syn::Error::new_spanned(
                    name,
                    "only one field may have `#[deref]`",
                )),
            }
        }
        Fields::Unnamed(f) => {
            let marked: Vec<_> = f
                .unnamed
                .iter()
                .enumerate()
                .filter(|(_, field)| field.attrs.iter().any(|a| a.path().is_ident("deref")))
                .collect();

            match marked.len() {
                0 => Err(syn::Error::new_spanned(
                    name,
                    "multiple fields require exactly one `#[deref]` attribute",
                )),
                1 => {
                    let (idx, field) = marked[0];
                    let ty = field.ty.clone();
                    let idx = syn::Index::from(idx);
                    let access = quote!(#idx);
                    Ok((ty, access))
                }
                _ => Err(syn::Error::new_spanned(
                    name,
                    "only one field may have `#[deref]`",
                )),
            }
        }
        Fields::Unit => Err(syn::Error::new_spanned(
            name,
            "Deref/DerefMut cannot be derived for unit structs",
        )),
    }
}
