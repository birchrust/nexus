//! `select!` proc macro — compile-time dispatch table for nexus-rt pipelines.
//!
//! Parses the macro body, detects the tier (1/2/3) and mode
//! (handler vs callback), and emits a closure with `resolve_step`
//! bindings and a literal `match`.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, ExprClosure, Pat, Token, Type};

// =============================================================================
// Custom keywords for named options
// =============================================================================

mod kw {
    syn::custom_keyword!(ctx);
    syn::custom_keyword!(key);
    syn::custom_keyword!(project);
}

// =============================================================================
// Parsed representation
// =============================================================================

pub struct SelectInput {
    reg: Expr,
    ctx_type: Option<Type>,
    key_closure: Option<ExprClosure>,
    project_closure: Option<ExprClosure>,
    arms: Vec<SelectArm>,
}

struct SelectArm {
    pat: Pat,
    handler: Expr,
    is_default: bool,
}

// =============================================================================
// Parsing
// =============================================================================

impl Parse for SelectInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        // 1. Registry expression, followed by comma
        let reg: Expr = input.parse()?;
        input.parse::<Token![,]>()?;

        // 2. Named options (ctx:, key:, project:) in any order
        let mut ctx_type = None;
        let mut key_closure = None;
        let mut project_closure = None;

        loop {
            if input.peek(kw::ctx) && input.peek2(Token![:]) {
                input.parse::<kw::ctx>()?;
                input.parse::<Token![:]>()?;
                ctx_type = Some(input.parse::<Type>()?);
                input.parse::<Token![,]>()?;
            } else if input.peek(kw::key) && input.peek2(Token![:]) {
                input.parse::<kw::key>()?;
                input.parse::<Token![:]>()?;
                key_closure = Some(input.parse::<ExprClosure>()?);
                input.parse::<Token![,]>()?;
            } else if input.peek(kw::project) && input.peek2(Token![:]) {
                let proj_kw = input.parse::<kw::project>()?;
                input.parse::<Token![:]>()?;
                if key_closure.is_none() {
                    return Err(syn::Error::new(
                        proj_kw.span,
                        "`project:` requires `key:` — cannot project without a key extraction",
                    ));
                }
                project_closure = Some(input.parse::<ExprClosure>()?);
                input.parse::<Token![,]>()?;
            } else {
                break;
            }
        }

        // 3. Arms
        let mut arms = Vec::new();
        let mut seen_default = false;

        while !input.is_empty() {
            if seen_default {
                return Err(input.error("default arm `_ =>` must be the last arm"));
            }

            let pat = Pat::parse_multi_with_leading_vert(input)?;
            input.parse::<Token![=>]>()?;
            let handler: Expr = input.parse()?;

            let is_default = matches!(&pat, Pat::Wild(_));

            if is_default {
                seen_default = true;
            }

            arms.push(SelectArm {
                pat,
                handler,
                is_default,
            });

            // Optional trailing comma
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        if arms.is_empty() {
            return Err(input.error("select! requires at least one arm"));
        }

        Ok(SelectInput {
            reg,
            ctx_type,
            key_closure,
            project_closure,
            arms,
        })
    }
}

// =============================================================================
// Code generation
// =============================================================================

pub fn expand(input: &SelectInput) -> TokenStream {
    if input.ctx_type.is_some() {
        emit_callback(input)
    } else {
        emit_handler(input)
    }
}

fn arm_ident(i: usize) -> syn::Ident {
    syn::Ident::new(&format!("__arm_{i}"), Span::mixed_site())
}

fn emit_handler(input: &SelectInput) -> TokenStream {
    let reg = &input.reg;
    let has_key = input.key_closure.is_some();
    let has_project = input.project_closure.is_some();

    // Generate arm bindings: let mut __arm_N = resolve_step(handler, reg);
    let mut arm_bindings = Vec::new();
    let mut match_arms = Vec::new();

    for (i, arm) in input.arms.iter().enumerate() {
        let arm_ident = arm_ident(i);
        let pat = &arm.pat;
        let handler = &arm.handler;

        if arm.is_default {
            // Default arm — inline directly in the match, no binding.
            // The handler is a closure |world, input| { ... } called in-place.
            if has_key {
                match_arms.push(quote! {
                    _ => (#handler)(__world, __input)
                });
            } else {
                match_arms.push(quote! {
                    __x => (#handler)(__world, __x)
                });
            }
        } else {
            arm_bindings.push(quote! {
                let mut #arm_ident = ::nexus_rt::resolve_step(#handler, #reg);
            });

            if has_key && has_project {
                // Tier 3: project the input for the arm
                let proj_fn = input.project_closure.as_ref().unwrap();
                match_arms.push(quote! {
                    #pat => {
                        let __projected = (#proj_fn)(__input);
                        #arm_ident(__world, __projected)
                    }
                });
            } else if has_key {
                // Tier 2: arms take the full input
                match_arms.push(quote! {
                    #pat => #arm_ident(__world, __input)
                });
            } else {
                // Tier 1: match on input directly, bind via @
                match_arms.push(quote! {
                    __x @ #pat => #arm_ident(__world, __x)
                });
            }
        }
    }

    // Build the closure body
    let closure_body = if has_key {
        let key_fn = input.key_closure.as_ref().unwrap();
        quote! {
            let __key = (#key_fn)(&__input);
            match __key {
                #(#match_arms,)*
            }
        }
    } else {
        quote! {
            match __input {
                #(#match_arms,)*
            }
        }
    };

    quote! {
        {
            #(#arm_bindings)*
            move |__world: &mut ::nexus_rt::World, __input| {
                #closure_body
            }
        }
    }
}

fn emit_callback(input: &SelectInput) -> TokenStream {
    let reg = &input.reg;
    let ctx_type = input.ctx_type.as_ref().unwrap();
    let has_key = input.key_closure.is_some();
    let has_project = input.project_closure.is_some();

    let mut arm_bindings = Vec::new();
    let mut match_arms = Vec::new();

    for (i, arm) in input.arms.iter().enumerate() {
        let arm_ident = arm_ident(i);
        let pat = &arm.pat;
        let handler = &arm.handler;

        if arm.is_default {
            // Default arm — inline directly, no binding.
            if has_key {
                match_arms.push(quote! {
                    _ => (#handler)(__ctx, __world, __input)
                });
            } else {
                match_arms.push(quote! {
                    __x => (#handler)(__ctx, __world, __x)
                });
            }
        } else {
            arm_bindings.push(quote! {
                let mut #arm_ident = ::nexus_rt::resolve_ctx_step::<#ctx_type, _, _, _, _>(#handler, #reg);
            });

            if has_key && has_project {
                let proj_fn = input.project_closure.as_ref().unwrap();
                match_arms.push(quote! {
                    #pat => {
                        let __projected = (#proj_fn)(__input);
                        #arm_ident(__ctx, __world, __projected)
                    }
                });
            } else if has_key {
                match_arms.push(quote! {
                    #pat => #arm_ident(__ctx, __world, __input)
                });
            } else {
                match_arms.push(quote! {
                    __x @ #pat => #arm_ident(__ctx, __world, __x)
                });
            }
        }
    }

    let closure_body = if has_key {
        let key_fn = input.key_closure.as_ref().unwrap();
        quote! {
            let __key = (#key_fn)(&__input);
            match __key {
                #(#match_arms,)*
            }
        }
    } else {
        quote! {
            match __input {
                #(#match_arms,)*
            }
        }
    };

    quote! {
        {
            #(#arm_bindings)*
            move |__ctx: &mut #ctx_type, __world: &mut ::nexus_rt::World, __input| {
                #closure_body
            }
        }
    }
}
