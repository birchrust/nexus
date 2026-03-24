//! Derive macros for nexus-rt.
//!
//! Use `nexus-rt` instead of depending on this crate directly.
//! The derives are re-exported from `nexus_rt::{Resource, Deref, DerefMut}`.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::visit_mut::VisitMut;
use syn::{Data, DeriveInput, Fields, Lifetime, parse_macro_input};

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

// =============================================================================
// #[derive(Param)]
// =============================================================================

/// Derive the `Param` trait for a struct, enabling it to be used as a
/// grouped handler parameter.
///
/// The struct must have exactly one lifetime parameter. Each field must
/// implement `Param`, or be annotated with `#[param(ignore)]` (in which
/// case it must implement `Default`).
///
/// ```ignore
/// use nexus_rt::{Param, Res, ResMut, Local};
///
/// #[derive(Param)]
/// struct TradingParams<'w> {
///     book: Res<'w, OrderBook>,
///     risk: ResMut<'w, RiskState>,
///     local_count: Local<'w, u64>,
/// }
///
/// fn on_order(params: TradingParams<'_>, order: Order) {
///     // params.book, params.risk, params.local_count all available
/// }
/// ```
#[proc_macro_derive(Param, attributes(param))]
pub fn derive_param(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_param_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn derive_param_impl(input: &DeriveInput) -> Result<proc_macro2::TokenStream, syn::Error> {
    let name = &input.ident;

    // Validate: must be a struct
    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "derive(Param) can only be applied to structs",
            ));
        }
    };

    // Validate: exactly one lifetime parameter
    let lifetimes: Vec<_> = input.generics.lifetimes().collect();
    if lifetimes.len() != 1 {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "derive(Param) requires exactly one lifetime parameter, \
             e.g., `struct MyParam<'w>`",
        ));
    }
    let world_lifetime = &lifetimes[0].lifetime;

    // Must be named fields
    let named_fields = match fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "derive(Param) requires named fields",
            ));
        }
    };

    // Classify fields: param fields (participate in init/fetch) vs ignored
    let mut param_fields = Vec::new();
    let mut ignored_fields = Vec::new();

    for field in named_fields {
        let field_name = field.ident.as_ref().unwrap();
        let is_ignored = field.attrs.iter().any(|a| {
            a.path().is_ident("param")
                && a.meta
                    .require_list()
                    .is_ok_and(|l| l.tokens.to_string().trim() == "ignore")
        });

        if is_ignored {
            ignored_fields.push(field_name);
        } else {
            // Substitute the struct's lifetime with 'static in the field type
            let mut static_ty = field.ty.clone();
            let mut replacer = LifetimeReplacer {
                from: world_lifetime.ident.to_string(),
            };
            replacer.visit_type_mut(&mut static_ty);

            param_fields.push((field_name, &field.ty, static_ty));
        }
    }

    // Generate the State struct name
    let state_name = format_ident!("{}State", name);

    // State struct fields
    let state_fields = param_fields.iter().map(|(field_name, _, static_ty)| {
        quote! {
            #field_name: <#static_ty as ::nexus_rt::Param>::State
        }
    });
    let ignored_state_fields = ignored_fields.iter().map(|field_name| {
        quote! {
            #field_name: ()
        }
    });

    // init() body
    let init_fields = param_fields.iter().map(|(field_name, _, static_ty)| {
        quote! {
            #field_name: <#static_ty as ::nexus_rt::Param>::init(registry)
        }
    });
    let init_ignored = ignored_fields.iter().map(|field_name| {
        quote! { #field_name: () }
    });

    // fetch() body
    let fetch_fields = param_fields.iter().map(|(field_name, _, static_ty)| {
        quote! {
            #field_name: <#static_ty as ::nexus_rt::Param>::fetch(world, &mut state.#field_name)
        }
    });
    let fetch_ignored = ignored_fields.iter().map(|field_name| {
        quote! {
            #field_name: ::core::default::Default::default()
        }
    });

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #state_name {
            #(#state_fields,)*
            #(#ignored_state_fields,)*
        }

        // SAFETY: Each field's State is Send (trait bound on Param::State).
        // The composed struct is therefore Send.
        unsafe impl Send for #state_name {}

        impl ::nexus_rt::Param for #name<'_> {
            type State = #state_name;
            type Item<'w> = #name<'w>;

            fn init(registry: &::nexus_rt::Registry) -> Self::State {
                #state_name {
                    #(#init_fields,)*
                    #(#init_ignored,)*
                }
            }

            unsafe fn fetch<'w>(
                world: &'w ::nexus_rt::World,
                state: &'w mut Self::State,
            ) -> #name<'w> {
                #name {
                    #(#fetch_fields,)*
                    #(#fetch_ignored,)*
                }
            }
        }
    })
}

/// Replaces occurrences of a specific lifetime with `'static`.
struct LifetimeReplacer {
    from: String,
}

impl VisitMut for LifetimeReplacer {
    fn visit_lifetime_mut(&mut self, lt: &mut Lifetime) {
        if lt.ident == self.from {
            *lt = Lifetime::new("'static", lt.apostrophe);
        }
    }
}
