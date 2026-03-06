// Builder return types are necessarily complex — each combinator returns
// PipelineBuilder<In, Out, impl FnMut(...)>. Same pattern as iterator adapters.
#![allow(clippy::type_complexity)]
// Audit functions intentionally have similar structure.
#![allow(clippy::too_many_lines)]
// Unused variables in closures that exist only as codegen probes.
#![allow(unused_variables)]
// Audit functions are not public API — no docs needed.
#![allow(missing_docs)]
// `world.registry()` returns `&Registry`; `&reg` is idiomatic in audit code.
#![allow(clippy::needless_borrow)]
// Audit helpers use short names (a, b, c) for resource params — intentional.
#![allow(clippy::many_single_char_names)]
// DAG steps take &u64 by design — we're auditing the reference-based codegen.
#![allow(clippy::trivially_copy_pass_by_ref)]
// Local fn items inside audit functions keep error handlers near usage.
#![allow(clippy::items_after_statements)]
// Audit helpers intentionally use approximate values.
#![allow(clippy::approx_constant)]
// `if b { 1 } else { 0 }` is the exact pattern we're auditing.
#![allow(clippy::bool_to_int_with_if)]
// Audit handler functions intentionally take params by value for codegen probing.
#![allow(clippy::needless_pass_by_value)]
// High-arity handlers are intentional — we're auditing the calling convention codegen.
#![allow(clippy::too_many_arguments)]

//! Assembly audit harness for verifying codegen quality.
//!
//! Every `pub fn` in this module is `#[inline(never)]` to provide
//! clean symbol boundaries for `cargo asm` inspection:
//!
//! ```bash
//! cargo asm --lib -p nexus-rt --features codegen-audit "pipe_linear_3"
//! ```
//!
//! These functions are NOT part of the public API. They exist solely
//! for codegen verification. The `codegen-audit` feature flag ensures
//! none of this compiles unless explicitly requested.

pub mod helpers;

mod pipeline;
pub use pipeline::*;

mod dag;
pub use dag::*;

mod batch;
pub use batch::*;

mod adapters;
pub use adapters::*;

mod stress;
pub use stress::*;
