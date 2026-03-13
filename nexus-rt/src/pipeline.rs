// Builder return types are necessarily complex — each combinator returns
// PipelineBuilder<In, Out, impl FnMut(...)>. Same pattern as iterator adapters.
#![allow(clippy::type_complexity)]

//! Pre-resolved pipeline dispatch using [`Param`] steps.
//!
//! [`PipelineStart`] begins a typed composition chain where each step
//! is a named function with [`Param`] dependencies resolved at build
//! time. The result is a monomorphized closure chain where dispatch-time
//! resource access is a single pointer deref per fetch — zero framework overhead.
//! [`ResourceId`](crate::ResourceId) is a direct pointer, not a HashMap lookup.
//!
//! Two dispatch tiers in nexus-rt:
//! 1. **Pipeline** — static after build, pre-resolved, the workhorse
//! 2. **Callback** — dynamic registration with per-instance context
//!
//! # Step function convention
//!
//! Params first, step input last, returns output:
//!
//! ```ignore
//! fn validate(config: Res<Config>, order: Order) -> Option<ValidOrder> { .. }
//! fn enrich(cache: Res<MarketData>, order: ValidOrder) -> EnrichedOrder { .. }
//! fn submit(mut gw: ResMut<Gateway>, order: CheckedOrder) { gw.send(order); }
//! ```
//!
//! # Combinator split
//!
//! **IntoStep-based (pre-resolved, hot path):**
//! `.then()`, `.map()`, `.and_then()`, `.catch()`
//!
//! **Trait-based (same API for named functions, arity-0 closures, and [`Opaque`] closures):**
//! `.guard()`, `.filter()`, `.tap()`, `.inspect()`, `.inspect_err()`,
//! `.on_none()`, `.ok_or_else()`, `.unwrap_or_else()`, `.map_err()`,
//! `.or_else()`, `.and()`, `.or()`, `.xor()`, `.route()`
//!
//! # Combinator quick reference
//!
//! **Bare value `T`:** `.then()`, `.tap()`, `.guard()` (→ `Option<T>`),
//! `.dispatch()`, `.route()`, `.tee()`, `.scan()`, `.dedup()` (→ `Option<T>`)
//!
//! **Tuple `(A, B, ...)` (2-5 elements):** `.splat()` (→ splat builder,
//! call `.then()` with destructured args)
//!
//! **`Option<T>`:** `.map()`, `.filter()`, `.inspect()`, `.and_then()`,
//! `.on_none()`, `.ok_or()` (→ `Result`), `.unwrap_or()` (→ `T`),
//! `.cloned()` (→ `Option<T>` from `Option<&T>`)
//!
//! **`Result<T, E>`:** `.map()`, `.and_then()`, `.catch()` (→ `Option<T>`),
//! `.map_err()`, `.inspect_err()`, `.ok()` (→ `Option<T>`),
//! `.unwrap_or()` (→ `T`), `.or_else()`
//!
//! **`bool`:** `.not()`, `.and()`, `.or()`, `.xor()`
//!
//! **Terminal:** `.build()` (→ `Pipeline`), `.build_batch(cap)`
//! (→ `BatchPipeline<In>`)
//!
//! # Splat — tuple destructuring
//!
//! Pipeline steps follow a single-value-in, single-value-out convention.
//! When a step returns a tuple like `(OrderId, f64)`, the next step
//! must accept the whole tuple as one argument. `.splat()` destructures
//! the tuple so the next step receives individual arguments instead:
//!
//! ```ignore
//! // Without splat — next step takes the whole tuple:
//! fn process(pair: (OrderId, f64)) -> bool { .. }
//!
//! // With splat — next step takes individual args:
//! fn process(id: OrderId, price: f64) -> bool { .. }
//!
//! PipelineStart::<Order>::new()
//!     .then(extract, reg)   // Order → (OrderId, f64)
//!     .splat()              // destructure
//!     .then(process, reg)   // (OrderId, f64) → bool
//!     .build();
//! ```
//!
//! Supported for tuples of 2-5 elements. Beyond 5, define a named
//! struct — if a combinator stage needs that many arguments, a struct
//! makes the intent clearer and the code more maintainable.

use std::marker::PhantomData;

use crate::Handler;
use crate::dag::DagArm;
use crate::handler::{Opaque, Param};
use crate::world::{Registry, World};

// =============================================================================
// Step — pre-resolved step with Param state
// =============================================================================

/// Internal: pre-resolved step with cached Param state.
///
/// Users don't construct this directly — it's produced by [`IntoStep`] and
/// captured inside pipeline chain closures.
#[doc(hidden)]
pub struct Step<F, Params: Param> {
    f: F,
    state: Params::State,
    #[allow(dead_code)]
    name: &'static str,
}

// =============================================================================
// StepCall — callable trait for resolved steps
// =============================================================================

/// Internal: callable trait for resolved steps.
///
/// Used as a bound on [`IntoStep::Step`]. Users don't implement this.
#[doc(hidden)]
pub trait StepCall<In, Out> {
    /// Call this step with a world reference and input value.
    fn call(&mut self, world: &mut World, input: In) -> Out;
}

// =============================================================================
// IntoStep — converts a named function into a resolved step
// =============================================================================

/// Converts a named function into a pre-resolved pipeline step.
///
/// Params first, step input last, returns output. Arity 0 (no
/// Params) supports closures. Arities 1+ require named functions
/// (same HRTB+GAT limitation as [`IntoHandler`](crate::IntoHandler)).
///
/// # Examples
///
/// ```ignore
/// // Arity 0 — closure works
/// let step = (|x: u32| x * 2).into_step(registry);
///
/// // Arity 1 — named function required
/// fn validate(config: Res<Config>, order: Order) -> Option<ValidOrder> { .. }
/// let step = validate.into_step(registry);
/// ```
pub trait IntoStep<In, Out, Params> {
    /// The concrete resolved step type.
    type Step: StepCall<In, Out>;

    /// Resolve Param state from the registry and produce a step.
    fn into_step(self, registry: &Registry) -> Self::Step;
}

// =============================================================================
// Arity 0 — fn(In) -> Out — closures work (no HRTB+GAT issues)
// =============================================================================

impl<In, Out, F: FnMut(In) -> Out + 'static> StepCall<In, Out> for Step<F, ()> {
    #[inline(always)]
    fn call(&mut self, _world: &mut World, input: In) -> Out {
        (self.f)(input)
    }
}

impl<In, Out, F: FnMut(In) -> Out + 'static> IntoStep<In, Out, ()> for F {
    type Step = Step<F, ()>;

    fn into_step(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

// =============================================================================
// Arities 1-8 via macro — HRTB with -> Out
// =============================================================================

macro_rules! impl_into_step {
    ($($P:ident),+) => {
        impl<In, Out, F: 'static, $($P: Param + 'static),+>
            StepCall<In, Out> for Step<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ In) -> Out +
                FnMut($($P::Item<'a>,)+ In) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call(&mut self, world: &mut World, input: In) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ Input, Output>(
                    mut f: impl FnMut($($P,)+ Input) -> Output,
                    $($P: $P,)+
                    input: Input,
                ) -> Output {
                    f($($P,)+ input)
                }

                // SAFETY: state was produced by init() on the same registry
                // that built this world. Single-threaded sequential dispatch
                // ensures no mutable aliasing across params.
                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ input)
            }
        }

        impl<In, Out, F: 'static, $($P: Param + 'static),+>
            IntoStep<In, Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ In) -> Out +
                FnMut($($P::Item<'a>,)+ In) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;

            fn into_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as Param>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

macro_rules! all_tuples {
    ($m:ident) => {
        $m!(P0);
        $m!(P0, P1);
        $m!(P0, P1, P2);
        $m!(P0, P1, P2, P3);
        $m!(P0, P1, P2, P3, P4);
        $m!(P0, P1, P2, P3, P4, P5);
        $m!(P0, P1, P2, P3, P4, P5, P6);
        $m!(P0, P1, P2, P3, P4, P5, P6, P7);
    };
}

all_tuples!(impl_into_step);

// =============================================================================
// OpaqueStep — wrapper for opaque closures as steps
// =============================================================================

/// Internal: wrapper for opaque closures used as pipeline steps.
///
/// Unlike [`Step<F, P>`] which stores resolved `Param::State`, this
/// holds only the function — no state to resolve.
#[doc(hidden)]
pub struct OpaqueStep<F> {
    f: F,
    #[allow(dead_code)]
    name: &'static str,
}

impl<In, Out, F: FnMut(&mut World, In) -> Out + 'static> StepCall<In, Out> for OpaqueStep<F> {
    #[inline(always)]
    fn call(&mut self, world: &mut World, input: In) -> Out {
        (self.f)(world, input)
    }
}

impl<In, Out, F: FnMut(&mut World, In) -> Out + 'static> IntoStep<In, Out, Opaque> for F {
    type Step = OpaqueStep<F>;

    fn into_step(self, _registry: &Registry) -> Self::Step {
        OpaqueStep {
            f: self,
            name: std::any::type_name::<F>(),
        }
    }
}

// =============================================================================
// RefStepCall / IntoRefStep — step taking &In, returning Out
// =============================================================================

/// Internal: callable trait for resolved steps taking input by reference.
///
/// Used by combinators like `tap`, `guard`, `filter`, `inspect` that
/// observe the value without consuming it.
#[doc(hidden)]
pub trait RefStepCall<In, Out> {
    /// Call this step with a world reference and borrowed input.
    fn call(&mut self, world: &mut World, input: &In) -> Out;
}

/// Converts a function into a pre-resolved step taking input by reference.
///
/// Same three-tier resolution as [`IntoStep`]:
///
/// | Params | Function shape | Example |
/// |--------|---------------|---------|
/// | `()` | `FnMut(&In) -> Out` | `\|o: &Order\| o.price > 0.0` |
/// | `(P0,)...(P0..P7,)` | `fn(Params..., &In) -> Out` | `fn check(c: Res<Config>, o: &Order) -> bool` |
/// | [`Opaque`] | `FnMut(&mut World, &In) -> Out` | `\|w: &mut World, o: &Order\| { ... }` |
pub trait IntoRefStep<In, Out, Params> {
    /// The concrete resolved step type.
    type Step: RefStepCall<In, Out>;

    /// Resolve Param state from the registry and produce a step.
    fn into_ref_step(self, registry: &Registry) -> Self::Step;
}

// -- Arity 0: FnMut(&In) -> Out — closures work ----------------------------

impl<In, Out, F: FnMut(&In) -> Out + 'static> RefStepCall<In, Out> for Step<F, ()> {
    #[inline(always)]
    fn call(&mut self, _world: &mut World, input: &In) -> Out {
        (self.f)(input)
    }
}

impl<In, Out, F: FnMut(&In) -> Out + 'static> IntoRefStep<In, Out, ()> for F {
    type Step = Step<F, ()>;

    fn into_ref_step(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

// -- Arities 1-8: named functions with Param resolution ---------------------

macro_rules! impl_into_ref_step {
    ($($P:ident),+) => {
        impl<In, Out, F: 'static, $($P: Param + 'static),+>
            RefStepCall<In, Out> for Step<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &In) -> Out +
                FnMut($($P::Item<'a>,)+ &In) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call(&mut self, world: &mut World, input: &In) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ Input: ?Sized, Output>(
                    mut f: impl FnMut($($P,)+ &Input) -> Output,
                    $($P: $P,)+
                    input: &Input,
                ) -> Output {
                    f($($P,)+ input)
                }

                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ input)
            }
        }

        impl<In, Out, F: 'static, $($P: Param + 'static),+>
            IntoRefStep<In, Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &In) -> Out +
                FnMut($($P::Item<'a>,)+ &In) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;

            fn into_ref_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as Param>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

all_tuples!(impl_into_ref_step);

// -- Opaque: FnMut(&mut World, &In) -> Out ---------------------------------

/// Internal: wrapper for opaque closures taking input by reference.
#[doc(hidden)]
pub struct OpaqueRefStep<F> {
    f: F,
    #[allow(dead_code)]
    name: &'static str,
}

impl<In, Out, F: FnMut(&mut World, &In) -> Out + 'static> RefStepCall<In, Out>
    for OpaqueRefStep<F>
{
    #[inline(always)]
    fn call(&mut self, world: &mut World, input: &In) -> Out {
        (self.f)(world, input)
    }
}

impl<In, Out, F: FnMut(&mut World, &In) -> Out + 'static> IntoRefStep<In, Out, Opaque> for F {
    type Step = OpaqueRefStep<F>;

    fn into_ref_step(self, _registry: &Registry) -> Self::Step {
        OpaqueRefStep {
            f: self,
            name: std::any::type_name::<F>(),
        }
    }
}

// =============================================================================
// resolve_ref_step — pre-resolve a ref step for manual dispatch
// =============================================================================

/// Resolve a reference step for manual dispatch.
///
/// Returns a closure with pre-resolved [`Param`] state. Reference-input
/// counterpart of [`resolve_step`].
pub fn resolve_ref_step<In, Out, Params, S: IntoRefStep<In, Out, Params>>(
    f: S,
    registry: &Registry,
) -> impl FnMut(&mut World, &In) -> Out + use<In, Out, Params, S> {
    let mut resolved = f.into_ref_step(registry);
    move |world: &mut World, input: &In| resolved.call(world, input)
}

// =============================================================================
// ProducerCall / IntoProducer — step producing a value with no pipeline input
// =============================================================================

/// Internal: callable trait for resolved steps that produce a value
/// without receiving pipeline input.
///
/// Used by combinators like `and`, `or`, `xor`, `on_none`, `ok_or_else`,
/// `unwrap_or_else` (Option).
#[doc(hidden)]
pub trait ProducerCall<Out> {
    /// Call this producer with a world reference.
    fn call(&mut self, world: &mut World) -> Out;
}

/// Converts a function into a pre-resolved producer step.
///
/// Same three-tier resolution as [`IntoStep`]:
///
/// | Params | Function shape | Example |
/// |--------|---------------|---------|
/// | `()` | `FnMut() -> Out` | `\|\| true` |
/// | `(P0,)...(P0..P7,)` | `fn(Params...) -> Out` | `fn is_active(s: Res<State>) -> bool` |
/// | [`Opaque`] | `FnMut(&mut World) -> Out` | `\|w: &mut World\| { ... }` |
pub trait IntoProducer<Out, Params> {
    /// The concrete resolved producer type.
    type Step: ProducerCall<Out>;

    /// Resolve Param state from the registry and produce a step.
    fn into_producer(self, registry: &Registry) -> Self::Step;
}

// -- Arity 0: FnMut() -> Out — closures work --------------------------------

impl<Out, F: FnMut() -> Out + 'static> ProducerCall<Out> for Step<F, ()> {
    #[inline(always)]
    fn call(&mut self, _world: &mut World) -> Out {
        (self.f)()
    }
}

impl<Out, F: FnMut() -> Out + 'static> IntoProducer<Out, ()> for F {
    type Step = Step<F, ()>;

    fn into_producer(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

// -- Arities 1-8: named functions with Param resolution ---------------------

macro_rules! impl_into_producer {
    ($($P:ident),+) => {
        impl<Out, F: 'static, $($P: Param + 'static),+>
            ProducerCall<Out> for Step<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+) -> Out +
                FnMut($($P::Item<'a>,)+) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call(&mut self, world: &mut World) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ Output>(
                    mut f: impl FnMut($($P,)+) -> Output,
                    $($P: $P,)+
                ) -> Output {
                    f($($P,)+)
                }

                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+)
            }
        }

        impl<Out, F: 'static, $($P: Param + 'static),+>
            IntoProducer<Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+) -> Out +
                FnMut($($P::Item<'a>,)+) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;

            fn into_producer(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as Param>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

all_tuples!(impl_into_producer);

// -- Opaque: FnMut(&mut World) -> Out ---------------------------------------

/// Internal: wrapper for opaque closures used as producers.
#[doc(hidden)]
pub struct OpaqueProducer<F> {
    f: F,
    #[allow(dead_code)]
    name: &'static str,
}

impl<Out, F: FnMut(&mut World) -> Out + 'static> ProducerCall<Out> for OpaqueProducer<F> {
    #[inline(always)]
    fn call(&mut self, world: &mut World) -> Out {
        (self.f)(world)
    }
}

impl<Out, F: FnMut(&mut World) -> Out + 'static> IntoProducer<Out, Opaque> for F {
    type Step = OpaqueProducer<F>;

    fn into_producer(self, _registry: &Registry) -> Self::Step {
        OpaqueProducer {
            f: self,
            name: std::any::type_name::<F>(),
        }
    }
}

// =============================================================================
// resolve_producer — pre-resolve a producer for manual dispatch
// =============================================================================

/// Resolve a producer for manual dispatch.
///
/// Returns a closure with pre-resolved [`Param`] state. No-input
/// counterpart of [`resolve_step`].
pub fn resolve_producer<Out, Params, S: IntoProducer<Out, Params>>(
    f: S,
    registry: &Registry,
) -> impl FnMut(&mut World) -> Out + use<Out, Params, S> {
    let mut resolved = f.into_producer(registry);
    move |world: &mut World| resolved.call(world)
}

// =============================================================================
// ScanStepCall / IntoScanStep — step with persistent accumulator
// =============================================================================

/// Internal: callable trait for resolved scan steps.
///
/// Like [`StepCall`] but with an additional `&mut Acc` accumulator
/// argument that persists across invocations.
#[doc(hidden)]
pub trait ScanStepCall<Acc, In, Out> {
    /// Call this scan step with a world reference, accumulator, and input value.
    fn call(&mut self, world: &mut World, acc: &mut Acc, input: In) -> Out;
}

/// Converts a function into a pre-resolved scan step with persistent state.
///
/// Same three-tier resolution as [`IntoStep`]:
///
/// | Params | Function shape | Example |
/// |--------|---------------|---------|
/// | `()` | `FnMut(&mut Acc, In) -> Out` | `\|acc, trade\| { *acc += trade.amount; Some(*acc) }` |
/// | `(P0,)...(P0..P7,)` | `fn(Params..., &mut Acc, In) -> Out` | `fn vwap(c: Res<Config>, acc: &mut State, t: Trade) -> Option<f64>` |
/// | [`Opaque`] | `FnMut(&mut World, &mut Acc, In) -> Out` | `\|w: &mut World, acc: &mut u64, t: Trade\| { ... }` |
pub trait IntoScanStep<Acc, In, Out, Params> {
    /// The concrete resolved scan step type.
    type Step: ScanStepCall<Acc, In, Out>;

    /// Resolve Param state from the registry and produce a scan step.
    fn into_scan_step(self, registry: &Registry) -> Self::Step;
}

// -- Arity 0: FnMut(&mut Acc, In) -> Out — closures work --------------------

impl<Acc, In, Out, F: FnMut(&mut Acc, In) -> Out + 'static>
    ScanStepCall<Acc, In, Out> for Step<F, ()>
{
    #[inline(always)]
    fn call(&mut self, _world: &mut World, acc: &mut Acc, input: In) -> Out {
        (self.f)(acc, input)
    }
}

impl<Acc, In, Out, F: FnMut(&mut Acc, In) -> Out + 'static>
    IntoScanStep<Acc, In, Out, ()> for F
{
    type Step = Step<F, ()>;

    fn into_scan_step(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

// -- Arities 1-8: named functions with Param resolution ----------------------

macro_rules! impl_into_scan_step {
    ($($P:ident),+) => {
        impl<Acc, In, Out, F: 'static, $($P: Param + 'static),+>
            ScanStepCall<Acc, In, Out> for Step<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &mut Acc, In) -> Out +
                FnMut($($P::Item<'a>,)+ &mut Acc, In) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call(&mut self, world: &mut World, acc: &mut Acc, input: In) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ Accumulator, Input, Output>(
                    mut f: impl FnMut($($P,)+ &mut Accumulator, Input) -> Output,
                    $($P: $P,)+
                    acc: &mut Accumulator,
                    input: Input,
                ) -> Output {
                    f($($P,)+ acc, input)
                }

                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ acc, input)
            }
        }

        impl<Acc, In, Out, F: 'static, $($P: Param + 'static),+>
            IntoScanStep<Acc, In, Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &mut Acc, In) -> Out +
                FnMut($($P::Item<'a>,)+ &mut Acc, In) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;

            fn into_scan_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as Param>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

all_tuples!(impl_into_scan_step);

// -- Opaque: FnMut(&mut World, &mut Acc, In) -> Out --------------------------

/// Internal: wrapper for opaque closures used as scan steps.
#[doc(hidden)]
pub struct OpaqueScanStep<F> {
    f: F,
    #[allow(dead_code)]
    name: &'static str,
}

impl<Acc, In, Out, F: FnMut(&mut World, &mut Acc, In) -> Out + 'static>
    ScanStepCall<Acc, In, Out> for OpaqueScanStep<F>
{
    #[inline(always)]
    fn call(&mut self, world: &mut World, acc: &mut Acc, input: In) -> Out {
        (self.f)(world, acc, input)
    }
}

impl<Acc, In, Out, F: FnMut(&mut World, &mut Acc, In) -> Out + 'static>
    IntoScanStep<Acc, In, Out, Opaque> for F
{
    type Step = OpaqueScanStep<F>;

    fn into_scan_step(self, _registry: &Registry) -> Self::Step {
        OpaqueScanStep {
            f: self,
            name: std::any::type_name::<F>(),
        }
    }
}

// =============================================================================
// resolve_scan_step — pre-resolve a scan step for manual dispatch
// =============================================================================

/// Resolve a scan step for manual dispatch.
///
/// Returns a closure with pre-resolved [`Param`] state. Scan variant
/// of [`resolve_step`] with an additional `&mut Acc` accumulator.
pub fn resolve_scan_step<Acc, In, Out, Params, S: IntoScanStep<Acc, In, Out, Params>>(
    f: S,
    registry: &Registry,
) -> impl FnMut(&mut World, &mut Acc, In) -> Out + use<Acc, In, Out, Params, S> {
    let mut resolved = f.into_scan_step(registry);
    move |world: &mut World, acc: &mut Acc, input: In| resolved.call(world, acc, input)
}

// =============================================================================
// RefScanStepCall / IntoRefScanStep — scan step taking &In
// =============================================================================

/// Internal: callable trait for resolved scan steps taking input by reference.
///
/// DAG variant of [`ScanStepCall`] — each step borrows its input.
#[doc(hidden)]
pub trait RefScanStepCall<Acc, In, Out> {
    /// Call this scan step with a world reference, accumulator, and borrowed input.
    fn call(&mut self, world: &mut World, acc: &mut Acc, input: &In) -> Out;
}

/// Converts a function into a pre-resolved ref-scan step with persistent state.
///
/// Same three-tier resolution as [`IntoRefStep`]:
///
/// | Params | Function shape | Example |
/// |--------|---------------|---------|
/// | `()` | `FnMut(&mut Acc, &In) -> Out` | `\|acc, trade: &Trade\| { ... }` |
/// | `(P0,)...(P0..P7,)` | `fn(Params..., &mut Acc, &In) -> Out` | `fn vwap(c: Res<Config>, acc: &mut State, t: &Trade) -> Option<f64>` |
/// | [`Opaque`] | `FnMut(&mut World, &mut Acc, &In) -> Out` | `\|w: &mut World, acc: &mut u64, t: &Trade\| { ... }` |
pub trait IntoRefScanStep<Acc, In, Out, Params> {
    /// The concrete resolved ref-scan step type.
    type Step: RefScanStepCall<Acc, In, Out>;

    /// Resolve Param state from the registry and produce a ref-scan step.
    fn into_ref_scan_step(self, registry: &Registry) -> Self::Step;
}

// -- Arity 0: FnMut(&mut Acc, &In) -> Out — closures work -------------------

impl<Acc, In, Out, F: FnMut(&mut Acc, &In) -> Out + 'static>
    RefScanStepCall<Acc, In, Out> for Step<F, ()>
{
    #[inline(always)]
    fn call(&mut self, _world: &mut World, acc: &mut Acc, input: &In) -> Out {
        (self.f)(acc, input)
    }
}

impl<Acc, In, Out, F: FnMut(&mut Acc, &In) -> Out + 'static>
    IntoRefScanStep<Acc, In, Out, ()> for F
{
    type Step = Step<F, ()>;

    fn into_ref_scan_step(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

// -- Arities 1-8: named functions with Param resolution ----------------------

macro_rules! impl_into_ref_scan_step {
    ($($P:ident),+) => {
        impl<Acc, In, Out, F: 'static, $($P: Param + 'static),+>
            RefScanStepCall<Acc, In, Out> for Step<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &mut Acc, &In) -> Out +
                FnMut($($P::Item<'a>,)+ &mut Acc, &In) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call(&mut self, world: &mut World, acc: &mut Acc, input: &In) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ Accumulator, Input: ?Sized, Output>(
                    mut f: impl FnMut($($P,)+ &mut Accumulator, &Input) -> Output,
                    $($P: $P,)+
                    acc: &mut Accumulator,
                    input: &Input,
                ) -> Output {
                    f($($P,)+ acc, input)
                }

                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ acc, input)
            }
        }

        impl<Acc, In, Out, F: 'static, $($P: Param + 'static),+>
            IntoRefScanStep<Acc, In, Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &mut Acc, &In) -> Out +
                FnMut($($P::Item<'a>,)+ &mut Acc, &In) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;

            fn into_ref_scan_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as Param>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

all_tuples!(impl_into_ref_scan_step);

// -- Opaque: FnMut(&mut World, &mut Acc, &In) -> Out ------------------------

/// Internal: wrapper for opaque closures used as ref-scan steps.
#[doc(hidden)]
pub struct OpaqueRefScanStep<F> {
    f: F,
    #[allow(dead_code)]
    name: &'static str,
}

impl<Acc, In, Out, F: FnMut(&mut World, &mut Acc, &In) -> Out + 'static>
    RefScanStepCall<Acc, In, Out> for OpaqueRefScanStep<F>
{
    #[inline(always)]
    fn call(&mut self, world: &mut World, acc: &mut Acc, input: &In) -> Out {
        (self.f)(world, acc, input)
    }
}

impl<Acc, In, Out, F: FnMut(&mut World, &mut Acc, &In) -> Out + 'static>
    IntoRefScanStep<Acc, In, Out, Opaque> for F
{
    type Step = OpaqueRefScanStep<F>;

    fn into_ref_scan_step(self, _registry: &Registry) -> Self::Step {
        OpaqueRefScanStep {
            f: self,
            name: std::any::type_name::<F>(),
        }
    }
}

// =============================================================================
// resolve_ref_scan_step — pre-resolve a ref-scan step for manual dispatch
// =============================================================================

/// Resolve a ref-scan step for manual dispatch.
///
/// Returns a closure with pre-resolved [`Param`] state. Reference-input
/// counterpart of [`resolve_scan_step`].
pub fn resolve_ref_scan_step<Acc, In, Out, Params, S: IntoRefScanStep<Acc, In, Out, Params>>(
    f: S,
    registry: &Registry,
) -> impl FnMut(&mut World, &mut Acc, &In) -> Out + use<Acc, In, Out, Params, S> {
    let mut resolved = f.into_ref_scan_step(registry);
    move |world: &mut World, acc: &mut Acc, input: &In| resolved.call(world, acc, input)
}

// =============================================================================
// SplatCall / IntoSplatStep — splat step dispatch (tuple destructuring)
// =============================================================================
//
// Splat traits mirror StepCall/IntoStep but accept multiple owned values
// instead of a single input. This lets `.splat()` destructure a tuple
// output into individual function arguments for the next step.
//
// One trait pair per arity (2-5). Past 5, use a named struct.

// -- Splat 2 ------------------------------------------------------------------

/// Internal: callable trait for resolved 2-splat steps.
#[doc(hidden)]
pub trait SplatCall2<A, B, Out> {
    fn call_splat(&mut self, world: &mut World, a: A, b: B) -> Out;
}

/// Converts a named function into a resolved 2-splat step.
#[doc(hidden)]
pub trait IntoSplatStep2<A, B, Out, Params> {
    type Step: SplatCall2<A, B, Out>;
    fn into_splat_step(self, registry: &Registry) -> Self::Step;
}

impl<A, B, Out, F: FnMut(A, B) -> Out + 'static> SplatCall2<A, B, Out> for Step<F, ()> {
    #[inline(always)]
    fn call_splat(&mut self, _world: &mut World, a: A, b: B) -> Out {
        (self.f)(a, b)
    }
}

impl<A, B, Out, F: FnMut(A, B) -> Out + 'static> IntoSplatStep2<A, B, Out, ()> for F {
    type Step = Step<F, ()>;
    fn into_splat_step(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

macro_rules! impl_splat2_step {
    ($($P:ident),+) => {
        impl<A, B, Out, F: 'static, $($P: Param + 'static),+>
            SplatCall2<A, B, Out> for Step<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ A, B) -> Out +
                FnMut($($P::Item<'a>,)+ A, B) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call_splat(&mut self, world: &mut World, a: A, b: B) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ IA, IB, Output>(
                    mut f: impl FnMut($($P,)+ IA, IB) -> Output,
                    $($P: $P,)+
                    a: IA, b: IB,
                ) -> Output {
                    f($($P,)+ a, b)
                }
                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ a, b)
            }
        }

        impl<A, B, Out, F: 'static, $($P: Param + 'static),+>
            IntoSplatStep2<A, B, Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ A, B) -> Out +
                FnMut($($P::Item<'a>,)+ A, B) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;

            fn into_splat_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as Param>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

// -- Splat 3 ------------------------------------------------------------------

/// Internal: callable trait for resolved 3-splat steps.
#[doc(hidden)]
pub trait SplatCall3<A, B, C, Out> {
    fn call_splat(&mut self, world: &mut World, a: A, b: B, c: C) -> Out;
}

/// Converts a named function into a resolved 3-splat step.
#[doc(hidden)]
pub trait IntoSplatStep3<A, B, C, Out, Params> {
    type Step: SplatCall3<A, B, C, Out>;
    fn into_splat_step(self, registry: &Registry) -> Self::Step;
}

impl<A, B, C, Out, F: FnMut(A, B, C) -> Out + 'static> SplatCall3<A, B, C, Out> for Step<F, ()> {
    #[inline(always)]
    fn call_splat(&mut self, _world: &mut World, a: A, b: B, c: C) -> Out {
        (self.f)(a, b, c)
    }
}

impl<A, B, C, Out, F: FnMut(A, B, C) -> Out + 'static> IntoSplatStep3<A, B, C, Out, ()> for F {
    type Step = Step<F, ()>;
    fn into_splat_step(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

macro_rules! impl_splat3_step {
    ($($P:ident),+) => {
        impl<A, B, C, Out, F: 'static, $($P: Param + 'static),+>
            SplatCall3<A, B, C, Out> for Step<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ A, B, C) -> Out +
                FnMut($($P::Item<'a>,)+ A, B, C) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call_splat(&mut self, world: &mut World, a: A, b: B, c: C) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ IA, IB, IC, Output>(
                    mut f: impl FnMut($($P,)+ IA, IB, IC) -> Output,
                    $($P: $P,)+
                    a: IA, b: IB, c: IC,
                ) -> Output {
                    f($($P,)+ a, b, c)
                }
                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ a, b, c)
            }
        }

        impl<A, B, C, Out, F: 'static, $($P: Param + 'static),+>
            IntoSplatStep3<A, B, C, Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ A, B, C) -> Out +
                FnMut($($P::Item<'a>,)+ A, B, C) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;

            fn into_splat_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as Param>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

// -- Splat 4 ------------------------------------------------------------------

/// Internal: callable trait for resolved 4-splat steps.
#[doc(hidden)]
pub trait SplatCall4<A, B, C, D, Out> {
    fn call_splat(&mut self, world: &mut World, a: A, b: B, c: C, d: D) -> Out;
}

/// Converts a named function into a resolved 4-splat step.
#[doc(hidden)]
pub trait IntoSplatStep4<A, B, C, D, Out, Params> {
    type Step: SplatCall4<A, B, C, D, Out>;
    fn into_splat_step(self, registry: &Registry) -> Self::Step;
}

impl<A, B, C, D, Out, F: FnMut(A, B, C, D) -> Out + 'static> SplatCall4<A, B, C, D, Out>
    for Step<F, ()>
{
    #[inline(always)]
    fn call_splat(&mut self, _world: &mut World, a: A, b: B, c: C, d: D) -> Out {
        (self.f)(a, b, c, d)
    }
}

impl<A, B, C, D, Out, F: FnMut(A, B, C, D) -> Out + 'static> IntoSplatStep4<A, B, C, D, Out, ()>
    for F
{
    type Step = Step<F, ()>;
    fn into_splat_step(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

macro_rules! impl_splat4_step {
    ($($P:ident),+) => {
        impl<A, B, C, D, Out, F: 'static, $($P: Param + 'static),+>
            SplatCall4<A, B, C, D, Out> for Step<F, ($($P,)+)>
        where for<'a> &'a mut F:
            FnMut($($P,)+ A, B, C, D) -> Out +
            FnMut($($P::Item<'a>,)+ A, B, C, D) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call_splat(&mut self, world: &mut World, a: A, b: B, c: C, d: D) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ IA, IB, IC, ID, Output>(
                    mut f: impl FnMut($($P,)+ IA, IB, IC, ID) -> Output,
                    $($P: $P,)+ a: IA, b: IB, c: IC, d: ID,
                ) -> Output { f($($P,)+ a, b, c, d) }
                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ a, b, c, d)
            }
        }
        impl<A, B, C, D, Out, F: 'static, $($P: Param + 'static),+>
            IntoSplatStep4<A, B, C, D, Out, ($($P,)+)> for F
        where for<'a> &'a mut F:
            FnMut($($P,)+ A, B, C, D) -> Out +
            FnMut($($P::Item<'a>,)+ A, B, C, D) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;
            fn into_splat_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                { #[allow(non_snake_case)] let ($($P,)+) = &state;
                  registry.check_access(&[$((<$P as Param>::resource_id($P), std::any::type_name::<$P>()),)+]); }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

// -- Splat 5 ------------------------------------------------------------------

/// Internal: callable trait for resolved 5-splat steps.
#[doc(hidden)]
pub trait SplatCall5<A, B, C, D, E, Out> {
    #[allow(clippy::many_single_char_names)]
    fn call_splat(&mut self, world: &mut World, a: A, b: B, c: C, d: D, e: E) -> Out;
}

/// Converts a named function into a resolved 5-splat step.
#[doc(hidden)]
pub trait IntoSplatStep5<A, B, C, D, E, Out, Params> {
    type Step: SplatCall5<A, B, C, D, E, Out>;
    fn into_splat_step(self, registry: &Registry) -> Self::Step;
}

impl<A, B, C, D, E, Out, F: FnMut(A, B, C, D, E) -> Out + 'static> SplatCall5<A, B, C, D, E, Out>
    for Step<F, ()>
{
    #[inline(always)]
    #[allow(clippy::many_single_char_names)]
    fn call_splat(&mut self, _world: &mut World, a: A, b: B, c: C, d: D, e: E) -> Out {
        (self.f)(a, b, c, d, e)
    }
}

impl<A, B, C, D, E, Out, F: FnMut(A, B, C, D, E) -> Out + 'static>
    IntoSplatStep5<A, B, C, D, E, Out, ()> for F
{
    type Step = Step<F, ()>;
    fn into_splat_step(self, registry: &Registry) -> Self::Step {
        Step {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

macro_rules! impl_splat5_step {
    ($($P:ident),+) => {
        impl<A, B, C, D, E, Out, F: 'static, $($P: Param + 'static),+>
            SplatCall5<A, B, C, D, E, Out> for Step<F, ($($P,)+)>
        where for<'a> &'a mut F:
            FnMut($($P,)+ A, B, C, D, E) -> Out +
            FnMut($($P::Item<'a>,)+ A, B, C, D, E) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case, clippy::many_single_char_names)]
            fn call_splat(&mut self, world: &mut World, a: A, b: B, c: C, d: D, e: E) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ IA, IB, IC, ID, IE, Output>(
                    mut f: impl FnMut($($P,)+ IA, IB, IC, ID, IE) -> Output,
                    $($P: $P,)+ a: IA, b: IB, c: IC, d: ID, e: IE,
                ) -> Output { f($($P,)+ a, b, c, d, e) }
                #[cfg(debug_assertions)]
                world.clear_borrows();
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ a, b, c, d, e)
            }
        }
        impl<A, B, C, D, E, Out, F: 'static, $($P: Param + 'static),+>
            IntoSplatStep5<A, B, C, D, E, Out, ($($P,)+)> for F
        where for<'a> &'a mut F:
            FnMut($($P,)+ A, B, C, D, E) -> Out +
            FnMut($($P::Item<'a>,)+ A, B, C, D, E) -> Out,
        {
            type Step = Step<F, ($($P,)+)>;
            fn into_splat_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as Param>::init(registry);
                { #[allow(non_snake_case)] let ($($P,)+) = &state;
                  registry.check_access(&[$((<$P as Param>::resource_id($P), std::any::type_name::<$P>()),)+]); }
                Step { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

all_tuples!(impl_splat2_step);
all_tuples!(impl_splat3_step);
all_tuples!(impl_splat4_step);
all_tuples!(impl_splat5_step);

// =============================================================================
// PipelineStart — entry point
// =============================================================================

/// Entry point for building a pre-resolved step pipeline.
///
/// `In` is the pipeline input type. Call [`.then()`](Self::then) to add
/// the first step — a named function whose [`Param`] dependencies
/// are resolved from the registry at build time.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, Res, ResMut, PipelineStart, Handler};
///
/// let mut wb = WorldBuilder::new();
/// wb.register::<u64>(10);
/// wb.register::<String>(String::new());
/// let mut world = wb.build();
///
/// fn double(factor: Res<u64>, x: u32) -> u64 {
///     (*factor) * x as u64
/// }
/// fn store(mut out: ResMut<String>, val: u64) {
///     *out = val.to_string();
/// }
///
/// let r = world.registry_mut();
/// let mut pipeline = PipelineStart::<u32>::new()
///     .then(double, r)
///     .then(store, r)
///     .build();
///
/// pipeline.run(&mut world, 5);
/// assert_eq!(world.resource::<String>().as_str(), "50");
/// ```
pub struct PipelineStart<In>(PhantomData<fn(In)>);

impl<In> PipelineStart<In> {
    /// Create a new step pipeline entry point.
    pub fn new() -> Self {
        Self(PhantomData)
    }

    /// Add the first step. Params resolved from the registry.
    pub fn then<Out, Params, S: IntoStep<In, Out, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Out, impl FnMut(&mut World, In) -> Out + use<In, Out, Params, S>> {
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| resolved.call(world, input),
            _marker: PhantomData,
        }
    }

    /// Add the first step as a scan with persistent accumulator.
    /// The step receives `&mut Acc` and the input, returning the output.
    /// State persists across invocations.
    pub fn scan<Acc, Out, Params, S>(
        self,
        initial: Acc,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Out, impl FnMut(&mut World, In) -> Out + use<In, Acc, Out, Params, S>>
    where
        Acc: 'static,
        S: IntoScanStep<Acc, In, Out, Params>,
    {
        let mut step = f.into_scan_step(registry);
        let mut acc = initial;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                step.call(world, &mut acc, input)
            },
            _marker: PhantomData,
        }
    }
}

impl<In> Default for PipelineStart<In> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// PipelineBuilder — typestate builder
// =============================================================================

/// Builder that composes pre-resolved pipeline steps via closure nesting.
///
/// `In` is the pipeline's input type (fixed). `Out` is the current output.
/// `Chain` is the concrete composed closure type (opaque, never named by users).
///
/// Each combinator consumes `self`, captures the previous chain in a new
/// closure, and returns a new `PipelineBuilder`. The compiler
/// monomorphizes the entire chain — zero virtual dispatch through steps.
///
/// IntoStep-based methods (`.then()`, `.map()`, `.and_then()`, `.catch()`)
/// take `&Registry` to resolve Param state at build time. Closure-based
/// methods don't need the registry.
pub struct PipelineBuilder<In, Out, Chain> {
    chain: Chain,
    _marker: PhantomData<fn(In) -> Out>,
}

// =============================================================================
// Core — any Out
// =============================================================================

impl<In, Out, Chain> PipelineBuilder<In, Out, Chain>
where
    Chain: FnMut(&mut World, In) -> Out,
{
    /// Add a step. Params resolved from the registry.
    pub fn then<NewOut, Params, S: IntoStep<Out, NewOut, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<
        In,
        NewOut,
        impl FnMut(&mut World, In) -> NewOut + use<In, Out, NewOut, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let out = chain(world, input);
                resolved.call(world, out)
            },
            _marker: PhantomData,
        }
    }

    /// Run the pipeline directly. No boxing, no `'static` on `In`.
    pub fn run(&mut self, world: &mut World, input: In) -> Out {
        (self.chain)(world, input)
    }

    /// Dispatch pipeline output to a [`Handler<Out>`].
    ///
    /// Connects a pipeline's output to any handler — [`HandlerFn`](crate::HandlerFn),
    /// [`Callback`](crate::Callback), [`Pipeline`], or a combinator like
    /// [`fan_out!`](crate::fan_out).
    pub fn dispatch<H: Handler<Out>>(
        self,
        mut handler: H,
    ) -> PipelineBuilder<In, (), impl FnMut(&mut World, In) + use<In, Out, Chain, H>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let out = chain(world, input);
                handler.run(world, out);
            },
            _marker: PhantomData,
        }
    }

    /// Conditionally wrap the output in `Option`. `Some(val)` if
    /// the predicate returns true, `None` otherwise.
    ///
    /// Enters Option-combinator land — follow with `.map()`,
    /// `.and_then()`, `.filter()`, `.unwrap_or()`, etc.
    pub fn guard<Params, S: IntoRefStep<Out, bool, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Option<Out>, impl FnMut(&mut World, In) -> Option<Out> + use<In, Out, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_ref_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let val = chain(world, input);
                if resolved.call(world, &val) { Some(val) } else { None }
            },
            _marker: PhantomData,
        }
    }

    /// Observe the current value without consuming or changing it.
    ///
    /// The step receives `&Out`. The value passes through unchanged.
    /// Useful for logging, metrics, or debugging mid-chain.
    pub fn tap<Params, S: IntoRefStep<Out, (), Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Out, impl FnMut(&mut World, In) -> Out + use<In, Out, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_ref_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let val = chain(world, input);
                resolved.call(world, &val);
                val
            },
            _marker: PhantomData,
        }
    }

    /// Binary conditional routing. Evaluates the predicate on the
    /// current value, then moves it into exactly one of two arms.
    ///
    /// Both arms must produce the same output type. Build each arm as
    /// a sub-pipeline from [`PipelineStart`]. For N-ary routing, nest
    /// `route` calls in the false arm.
    ///
    /// ```ignore
    /// let large = PipelineStart::new().then(large_check, reg).then(submit, reg);
    /// let small = PipelineStart::new().then(submit, reg);
    ///
    /// PipelineStart::<Order>::new()
    ///     .then(validate, reg)
    ///     .route(|order: &Order| order.size > 1000, reg, large, small)
    ///     .build();
    /// ```
    pub fn route<NewOut, C0, C1, Params, Pred: IntoRefStep<Out, bool, Params>>(
        self,
        pred: Pred,
        registry: &Registry,
        on_true: PipelineBuilder<Out, NewOut, C0>,
        on_false: PipelineBuilder<Out, NewOut, C1>,
    ) -> PipelineBuilder<
        In,
        NewOut,
        impl FnMut(&mut World, In) -> NewOut + use<In, Out, NewOut, Params, Chain, C0, C1, Pred>,
    >
    where
        C0: FnMut(&mut World, Out) -> NewOut,
        C1: FnMut(&mut World, Out) -> NewOut,
    {
        let mut chain = self.chain;
        let mut resolved = pred.into_ref_step(registry);
        let mut c0 = on_true.chain;
        let mut c1 = on_false.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let val = chain(world, input);
                if resolved.call(world, &val) {
                    c0(world, val)
                } else {
                    c1(world, val)
                }
            },
            _marker: PhantomData,
        }
    }

    /// Fork off a multi-step side-effect chain. The arm borrows
    /// `&Out`, runs to completion (producing `()`), and the
    /// original value passes through unchanged.
    ///
    /// Multi-step version of [`tap`](Self::tap) — the arm has the
    /// full DAG combinator API with Param resolution. Build with
    /// [`DagArmStart::new()`](crate::dag::DagArmStart::new).
    pub fn tee<C>(
        self,
        side: DagArm<Out, (), C>,
    ) -> PipelineBuilder<In, Out, impl FnMut(&mut World, In) -> Out>
    where
        C: FnMut(&mut World, &Out),
    {
        let mut chain = self.chain;
        let mut side_chain = side.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let val = chain(world, input);
                side_chain(world, &val);
                val
            },
            _marker: PhantomData,
        }
    }

    /// Scan with persistent accumulator. The step receives `&mut Acc`
    /// and the current value, returning the new output. State persists
    /// across invocations.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Running sum — suppress values below threshold
    /// PipelineStart::<u64>::new()
    ///     .then(identity, reg)
    ///     .scan(0u64, |acc: &mut u64, val: u64| {
    ///         *acc += val;
    ///         if *acc > 100 { Some(*acc) } else { None }
    ///     }, reg)
    ///     .build();
    /// ```
    pub fn scan<Acc, NewOut, Params, S>(
        self,
        initial: Acc,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<
        In,
        NewOut,
        impl FnMut(&mut World, In) -> NewOut + use<In, Out, Acc, NewOut, Params, S, Chain>,
    >
    where
        Acc: 'static,
        S: IntoScanStep<Acc, Out, NewOut, Params>,
    {
        let mut chain = self.chain;
        let mut step = f.into_scan_step(registry);
        let mut acc = initial;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let val = chain(world, input);
                step.call(world, &mut acc, val)
            },
            _marker: PhantomData,
        }
    }

}

// =============================================================================
// Splat — tuple destructuring into individual function arguments
// =============================================================================
//
// `.splat()` transitions from a tuple output to a builder whose `.then()`
// accepts a function taking the tuple elements as individual arguments.
// After `.splat().then(f, reg)`, the user is back on PipelineBuilder.
//
// Builder types are `#[doc(hidden)]` — users only see `.splat().then()`.

// -- Splat builder types ------------------------------------------------------

macro_rules! define_splat_builders {
    (
        $N:literal,
        start: $SplatStart:ident,
        mid: $SplatBuilder:ident,
        into_trait: $IntoSplatStep:ident,
        call_trait: $SplatCall:ident,
        ($($T:ident),+),
        ($($idx:tt),+)
    ) => {
        /// Splat builder at pipeline start position.
        #[doc(hidden)]
        pub struct $SplatStart<$($T),+>(PhantomData<fn(($($T,)+))>);

        impl<$($T),+> $SplatStart<$($T),+> {
            /// Add a step that receives the tuple elements as individual arguments.
            pub fn then<Out, Params, S>(
                self,
                f: S,
                registry: &Registry,
            ) -> PipelineBuilder<
                ($($T,)+),
                Out,
                impl FnMut(&mut World, ($($T,)+)) -> Out
                    + use<$($T,)+ Out, Params, S>,
            >
            where
                S: $IntoSplatStep<$($T,)+ Out, Params>,
            {
                let mut resolved = f.into_splat_step(registry);
                PipelineBuilder {
                    chain: move |world: &mut World, input: ($($T,)+)| {
                        resolved.call_splat(world, $(input.$idx),+)
                    },
                    _marker: PhantomData,
                }
            }
        }

        impl<$($T),+> PipelineStart<($($T,)+)> {
            /// Destructure the tuple input into individual function arguments.
            pub fn splat(self) -> $SplatStart<$($T),+> {
                $SplatStart(PhantomData)
            }
        }

        /// Splat builder at mid-chain position.
        #[doc(hidden)]
        pub struct $SplatBuilder<In, $($T,)+ Chain> {
            chain: Chain,
            _marker: PhantomData<fn(In) -> ($($T,)+)>,
        }

        impl<In, $($T,)+ Chain> $SplatBuilder<In, $($T,)+ Chain>
        where
            Chain: FnMut(&mut World, In) -> ($($T,)+),
        {
            /// Add a step that receives the tuple elements as individual arguments.
            pub fn then<Out, Params, S>(
                self,
                f: S,
                registry: &Registry,
            ) -> PipelineBuilder<
                In,
                Out,
                impl FnMut(&mut World, In) -> Out
                    + use<In, $($T,)+ Out, Params, Chain, S>,
            >
            where
                S: $IntoSplatStep<$($T,)+ Out, Params>,
            {
                let mut chain = self.chain;
                let mut resolved = f.into_splat_step(registry);
                PipelineBuilder {
                    chain: move |world: &mut World, input: In| {
                        let tuple = chain(world, input);
                        resolved.call_splat(world, $(tuple.$idx),+)
                    },
                    _marker: PhantomData,
                }
            }
        }

        impl<In, $($T,)+ Chain> PipelineBuilder<In, ($($T,)+), Chain>
        where
            Chain: FnMut(&mut World, In) -> ($($T,)+),
        {
            /// Destructure the tuple output into individual function arguments.
            pub fn splat(self) -> $SplatBuilder<In, $($T,)+ Chain> {
                $SplatBuilder {
                    chain: self.chain,
                    _marker: PhantomData,
                }
            }
        }
    };
}

define_splat_builders!(2,
    start: SplatStart2,
    mid: SplatBuilder2,
    into_trait: IntoSplatStep2,
    call_trait: SplatCall2,
    (A, B),
    (0, 1)
);

define_splat_builders!(3,
    start: SplatStart3,
    mid: SplatBuilder3,
    into_trait: IntoSplatStep3,
    call_trait: SplatCall3,
    (A, B, C),
    (0, 1, 2)
);

define_splat_builders!(4,
    start: SplatStart4,
    mid: SplatBuilder4,
    into_trait: IntoSplatStep4,
    call_trait: SplatCall4,
    (A, B, C, D),
    (0, 1, 2, 3)
);

define_splat_builders!(5,
    start: SplatStart5,
    mid: SplatBuilder5,
    into_trait: IntoSplatStep5,
    call_trait: SplatCall5,
    (A, B, C, D, E),
    (0, 1, 2, 3, 4)
);

// =============================================================================
// Dedup — suppress unchanged values
// =============================================================================

impl<In, Out: PartialEq + Clone, Chain> PipelineBuilder<In, Out, Chain>
where
    Chain: FnMut(&mut World, In) -> Out,
{
    /// Suppress consecutive unchanged values. Returns `Some(val)`
    /// when the value differs from the previous invocation, `None`
    /// when unchanged. First invocation always returns `Some`.
    ///
    /// Requires `PartialEq + Clone` — the previous value is stored
    /// internally for comparison.
    pub fn dedup(
        self,
    ) -> PipelineBuilder<In, Option<Out>, impl FnMut(&mut World, In) -> Option<Out>> {
        let mut chain = self.chain;
        let mut prev: Option<Out> = None;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let val = chain(world, input);
                if prev.as_ref() == Some(&val) {
                    None
                } else {
                    prev = Some(val.clone());
                    Some(val)
                }
            },
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Bool combinators
// =============================================================================

impl<In, Chain> PipelineBuilder<In, bool, Chain>
where
    Chain: FnMut(&mut World, In) -> bool,
{
    /// Invert a boolean value.
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> PipelineBuilder<In, bool, impl FnMut(&mut World, In) -> bool> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| !chain(world, input),
            _marker: PhantomData,
        }
    }

    /// Short-circuit AND with a second boolean.
    ///
    /// If the chain produces `false`, the step is not called.
    pub fn and<Params, S: IntoProducer<bool, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, bool, impl FnMut(&mut World, In) -> bool + use<In, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_producer(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input) && resolved.call(world)
            },
            _marker: PhantomData,
        }
    }

    /// Short-circuit OR with a second boolean.
    ///
    /// If the chain produces `true`, the step is not called.
    pub fn or<Params, S: IntoProducer<bool, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, bool, impl FnMut(&mut World, In) -> bool + use<In, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_producer(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input) || resolved.call(world)
            },
            _marker: PhantomData,
        }
    }

    /// XOR with a second boolean.
    ///
    /// Both sides are always evaluated.
    pub fn xor<Params, S: IntoProducer<bool, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, bool, impl FnMut(&mut World, In) -> bool + use<In, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_producer(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input) ^ resolved.call(world)
            },
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Clone helpers — &T → T transitions
// =============================================================================

impl<'a, In, T: Clone, Chain> PipelineBuilder<In, &'a T, Chain>
where
    Chain: FnMut(&mut World, In) -> &'a T,
{
    /// Clone a borrowed output to produce an owned value.
    ///
    /// Transitions the pipeline from `&T` to `T`. Uses UFCS
    /// (`T::clone(val)`) — `val.clone()` on a `&&T` resolves to
    /// `<&T as Clone>::clone` and returns `&T`, not `T`.
    pub fn cloned(self) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| T::clone(chain(world, input)),
            _marker: PhantomData,
        }
    }
}

impl<'a, In, T: Clone, Chain> PipelineBuilder<In, Option<&'a T>, Chain>
where
    Chain: FnMut(&mut World, In) -> Option<&'a T>,
{
    /// Clone inner borrowed value. `Option<&T>` → `Option<T>`.
    pub fn cloned(self) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| chain(world, input).cloned(),
            _marker: PhantomData,
        }
    }
}

impl<'a, In, T: Clone, E, Chain> PipelineBuilder<In, Result<&'a T, E>, Chain>
where
    Chain: FnMut(&mut World, In) -> Result<&'a T, E>,
{
    /// Clone inner borrowed Ok value. `Result<&T, E>` → `Result<T, E>`.
    pub fn cloned(
        self,
    ) -> PipelineBuilder<In, Result<T, E>, impl FnMut(&mut World, In) -> Result<T, E>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| chain(world, input).cloned(),
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Option helpers — PipelineBuilder<In, Option<T>, Chain>
// =============================================================================

impl<In, T, Chain> PipelineBuilder<In, Option<T>, Chain>
where
    Chain: FnMut(&mut World, In) -> Option<T>,
{
    // -- IntoStep-based (hot path) -------------------------------------------

    /// Transform the inner value. Step not called on None.
    pub fn map<U, Params, S: IntoStep<T, U, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<
        In,
        Option<U>,
        impl FnMut(&mut World, In) -> Option<U> + use<In, T, U, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).map(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Short-circuits on None. std: `Option::and_then`
    pub fn and_then<U, Params, S: IntoStep<T, Option<U>, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<
        In,
        Option<U>,
        impl FnMut(&mut World, In) -> Option<U> + use<In, T, U, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).and_then(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    // -- Resolved (cold path, now with Param resolution) -----------------------

    /// Side effect on None. Complement to [`inspect`](Self::inspect).
    pub fn on_none<Params, S: IntoProducer<(), Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T> + use<In, T, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_producer(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let result = chain(world, input);
                if result.is_none() {
                    resolved.call(world);
                }
                result
            },
            _marker: PhantomData,
        }
    }

    /// Keep value if predicate holds. std: `Option::filter`
    pub fn filter<Params, S: IntoRefStep<T, bool, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T> + use<In, T, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_ref_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).filter(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Side effect on Some value. std: `Option::inspect`
    pub fn inspect<Params, S: IntoRefStep<T, (), Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T> + use<In, T, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_ref_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).inspect(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// None becomes Err(err). std: `Option::ok_or`
    ///
    /// `Clone` required because the pipeline may run many times —
    /// the error value is cloned on each `None` invocation.
    pub fn ok_or<E: Clone>(
        self,
        err: E,
    ) -> PipelineBuilder<In, Result<T, E>, impl FnMut(&mut World, In) -> Result<T, E>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).ok_or_else(|| err.clone())
            },
            _marker: PhantomData,
        }
    }

    /// None becomes Err(f()). std: `Option::ok_or_else`
    pub fn ok_or_else<E, Params, S: IntoProducer<E, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Result<T, E>, impl FnMut(&mut World, In) -> Result<T, E> + use<In, T, E, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_producer(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).ok_or_else(|| resolved.call(world))
            },
            _marker: PhantomData,
        }
    }

    /// Exit Option — None becomes the default value.
    ///
    /// `Clone` required because the pipeline may run many times —
    /// the default is cloned on each `None` invocation (unlike
    /// std's `unwrap_or` which consumes the value once).
    pub fn unwrap_or(self, default: T) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T>
    where
        T: Clone,
    {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).unwrap_or_else(|| default.clone())
            },
            _marker: PhantomData,
        }
    }

    /// Exit Option — None becomes `f()`.
    pub fn unwrap_or_else<Params, S: IntoProducer<T, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T + use<In, T, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_producer(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).unwrap_or_else(|| resolved.call(world))
            },
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Result helpers — PipelineBuilder<In, Result<T, E>, Chain>
// =============================================================================

impl<In, T, E, Chain> PipelineBuilder<In, Result<T, E>, Chain>
where
    Chain: FnMut(&mut World, In) -> Result<T, E>,
{
    // -- IntoStep-based (hot path) -------------------------------------------

    /// Transform the Ok value. Step not called on Err.
    pub fn map<U, Params, S: IntoStep<T, U, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<
        In,
        Result<U, E>,
        impl FnMut(&mut World, In) -> Result<U, E> + use<In, T, E, U, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).map(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Short-circuits on Err. std: `Result::and_then`
    pub fn and_then<U, Params, S: IntoStep<T, Result<U, E>, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<
        In,
        Result<U, E>,
        impl FnMut(&mut World, In) -> Result<U, E> + use<In, T, E, U, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).and_then(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Handle error and transition to Option.
    ///
    /// `Ok(val)` becomes `Some(val)` — handler not called.
    /// `Err(err)` calls the handler, then produces `None`.
    pub fn catch<Params, S: IntoStep<E, (), Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<
        In,
        Option<T>,
        impl FnMut(&mut World, In) -> Option<T> + use<In, T, E, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| match chain(world, input) {
                Ok(val) => Some(val),
                Err(err) => {
                    resolved.call(world, err);
                    None
                }
            },
            _marker: PhantomData,
        }
    }

    // -- Resolved (cold path, now with Param resolution) -----------------------

    /// Transform the error. std: `Result::map_err`
    pub fn map_err<E2, Params, S: IntoStep<E, E2, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Result<T, E2>, impl FnMut(&mut World, In) -> Result<T, E2> + use<In, T, E, E2, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).map_err(|err| resolved.call(world, err))
            },
            _marker: PhantomData,
        }
    }

    /// Recover from Err. std: `Result::or_else`
    pub fn or_else<E2, Params, S: IntoStep<E, Result<T, E2>, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Result<T, E2>, impl FnMut(&mut World, In) -> Result<T, E2> + use<In, T, E, E2, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).or_else(|err| resolved.call(world, err))
            },
            _marker: PhantomData,
        }
    }

    /// Side effect on Ok. std: `Result::inspect`
    pub fn inspect<Params, S: IntoRefStep<T, (), Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Result<T, E>, impl FnMut(&mut World, In) -> Result<T, E> + use<In, T, E, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_ref_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).inspect(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Side effect on Err. std: `Result::inspect_err`
    pub fn inspect_err<Params, S: IntoRefStep<E, (), Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, Result<T, E>, impl FnMut(&mut World, In) -> Result<T, E> + use<In, T, E, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_ref_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).inspect_err(|err| resolved.call(world, err))
            },
            _marker: PhantomData,
        }
    }

    /// Discard error, enter Option land. std: `Result::ok`
    pub fn ok(self) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| chain(world, input).ok(),
            _marker: PhantomData,
        }
    }

    /// Exit Result — Err becomes the default value.
    ///
    /// `Clone` required because the pipeline may run many times —
    /// the default is cloned on each `Err` invocation (unlike
    /// std's `unwrap_or` which consumes the value once).
    pub fn unwrap_or(self, default: T) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T>
    where
        T: Clone,
    {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).unwrap_or_else(|_| default.clone())
            },
            _marker: PhantomData,
        }
    }

    /// Exit Result — Err becomes `f(err)`.
    pub fn unwrap_or_else<Params, S: IntoStep<E, T, Params>>(
        self,
        f: S,
        registry: &Registry,
    ) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T + use<In, T, E, Params, S, Chain>> {
        let mut chain = self.chain;
        let mut resolved = f.into_step(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| match chain(world, input) {
                Ok(val) => val,
                Err(err) => resolved.call(world, err),
            },
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// PipelineOutput — marker trait for build()
// =============================================================================

/// Marker trait restricting [`PipelineBuilder::build`] to pipelines
/// that produce `()`.
///
/// If your pipeline produces a value, add a final `.then()` that
/// writes it somewhere (e.g. `ResMut<T>`).
#[diagnostic::on_unimplemented(
    message = "`build()` requires the step pipeline output to be `()`",
    label = "this pipeline produces `{Self}`, not `()`",
    note = "add a final `.then()` that consumes the output"
)]
pub trait PipelineOutput {}
impl PipelineOutput for () {}
impl PipelineOutput for Option<()> {}

// =============================================================================
// build — when Out: PipelineOutput (() or Option<()>)
// =============================================================================

impl<In, Chain> PipelineBuilder<In, (), Chain>
where
    Chain: FnMut(&mut World, In),
{
    /// Finalize the pipeline into a [`Pipeline`].
    ///
    /// The returned pipeline is a concrete, monomorphized type — no boxing,
    /// no virtual dispatch. Call `.run()` directly for zero-cost execution,
    /// or wrap in `Box<dyn Handler<In>>` when type erasure is needed.
    ///
    /// Only available when the pipeline ends with `()` or `Option<()>`.
    /// If your chain produces a value, add a final `.then()` that consumes
    /// the output.
    pub fn build(self) -> Pipeline<Chain> {
        Pipeline {
            chain: self.chain,
        }
    }
}

impl<In, Chain> PipelineBuilder<In, Option<()>, Chain>
where
    Chain: FnMut(&mut World, In) -> Option<()>,
{
    /// Finalize the pipeline into a [`Pipeline`], discarding the `Option<()>`.
    ///
    /// Pipelines ending with `Option<()>` (e.g. after `.map()` on an
    /// `Option<T>` with a step that returns `()`) produce the same
    /// [`Pipeline`] as those ending with `()`.
    pub fn build(self) -> Pipeline<impl FnMut(&mut World, In) + use<In, Chain>> {
        let mut chain = self.chain;
        Pipeline {
            chain: move |world: &mut World, input: In| {
                let _ = chain(world, input);
            },
        }
    }
}

// =============================================================================
// build_batch — when Out: PipelineOutput (() or Option<()>)
// =============================================================================

impl<In, Out: PipelineOutput, Chain> PipelineBuilder<In, Out, Chain>
where
    Chain: FnMut(&mut World, In) -> Out,
{
    /// Finalize into a [`BatchPipeline`] with a pre-allocated input buffer.
    ///
    /// Same pipeline chain as [`build`](PipelineBuilder::build), but the
    /// pipeline owns an input buffer that drivers fill between dispatch
    /// cycles. Each call to [`BatchPipeline::run`] drains the buffer,
    /// running every item through the chain independently.
    ///
    /// Available when the pipeline ends with `()` or `Option<()>` (e.g.
    /// after `.catch()` or `.filter()`). Pipelines producing values need
    /// a final `.then()` that consumes the output.
    ///
    /// `capacity` is the initial allocation — the buffer can grow if needed,
    /// but sizing it for the expected batch size avoids reallocation.
    pub fn build_batch(self, capacity: usize) -> BatchPipeline<In, Chain> {
        BatchPipeline {
            input: Vec::with_capacity(capacity),
            chain: self.chain,
        }
    }
}

// =============================================================================
// Pipeline<F> — built pipeline
// =============================================================================

/// Built step pipeline implementing [`Handler<E>`](crate::Handler).
///
/// Created by [`PipelineBuilder::build`]. The entire pipeline chain is
/// monomorphized at compile time — no boxing, no virtual dispatch.
/// Call `.run()` directly for zero-cost execution, or wrap in
/// `Box<dyn Handler<E>>` when you need type erasure (single box).
///
/// Implements [`Handler<E>`](crate::Handler) for any event type `E`
/// that the chain accepts — including borrowed types like `&'a [u8]`.
/// Supports `for<'a> Handler<&'a T>` for zero-copy event dispatch.
pub struct Pipeline<F> {
    chain: F,
}

impl<E, F: FnMut(&mut World, E) + Send> crate::Handler<E> for Pipeline<F> {
    fn run(&mut self, world: &mut World, event: E) {
        (self.chain)(world, event);
    }
}

// =============================================================================
// BatchPipeline<In, F> — pipeline with owned input buffer
// =============================================================================

/// Batch pipeline that owns a pre-allocated input buffer.
///
/// Created by [`PipelineBuilder::build_batch`]. Each item flows through
/// the full pipeline chain independently — the same per-item `Option`
/// and `Result` flow control as [`Pipeline`]. Errors are handled inline
/// (via `.catch()`, `.unwrap_or()`, etc.) and the batch continues to
/// the next item. No intermediate buffers between steps.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, Res, ResMut, PipelineStart};
///
/// let mut wb = WorldBuilder::new();
/// wb.register::<u64>(0);
/// let mut world = wb.build();
///
/// fn accumulate(mut sum: ResMut<u64>, x: u32) {
///     *sum += x as u64;
/// }
///
/// let r = world.registry_mut();
/// let mut batch = PipelineStart::<u32>::new()
///     .then(accumulate, r)
///     .build_batch(64);
///
/// batch.input_mut().extend_from_slice(&[1, 2, 3, 4, 5]);
/// batch.run(&mut world);
///
/// assert_eq!(*world.resource::<u64>(), 15);
/// assert!(batch.input().is_empty());
/// ```
pub struct BatchPipeline<In, F> {
    input: Vec<In>,
    chain: F,
}

impl<In, Out: PipelineOutput, F: FnMut(&mut World, In) -> Out> BatchPipeline<In, F> {
    /// Mutable access to the input buffer. Drivers fill this between
    /// dispatch cycles.
    pub fn input_mut(&mut self) -> &mut Vec<In> {
        &mut self.input
    }

    /// Read-only access to the input buffer.
    pub fn input(&self) -> &[In] {
        &self.input
    }

    /// Drain the input buffer, running each item through the pipeline.
    ///
    /// Each item gets independent `Option`/`Result` flow control — an
    /// error on one item does not affect subsequent items. After `run()`,
    /// the input buffer is empty but retains its allocation.
    pub fn run(&mut self, world: &mut World) {
        for item in self.input.drain(..) {
            let _ = (self.chain)(world, item);
        }
    }
}

// =============================================================================
// resolve_step — pre-resolve a step for manual dispatch (owned input)
// =============================================================================

/// Resolve a step for use in manual dispatch (e.g. inside an
/// [`Opaque`] closure passed to `.then()`).
///
/// Returns a closure with pre-resolved [`Param`] state — the same
/// build-time resolution that `.then()` performs, but as a standalone
/// value the caller can invoke from any context.
///
/// This is the pipeline (owned-input) counterpart of
/// [`dag::resolve_arm`](crate::dag::resolve_arm) (reference-input).
///
/// # Examples
///
/// ```ignore
/// let mut arm0 = resolve_step(handle_new, &reg);
/// let mut arm1 = resolve_step(handle_cancel, &reg);
///
/// pipeline.then(move |world: &mut World, order: Order| match order.kind {
///     OrderKind::New    => arm0(world, order),
///     OrderKind::Cancel => arm1(world, order),
/// }, &reg)
/// ```
pub fn resolve_step<In, Out, Params, S>(
    f: S,
    registry: &Registry,
) -> impl FnMut(&mut World, In) -> Out + use<In, Out, Params, S>
where
    In: 'static,
    Out: 'static,
    S: IntoStep<In, Out, Params>,
{
    let mut resolved = f.into_step(registry);
    move |world: &mut World, input: In| resolved.call(world, input)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Handler, IntoHandler, Local, Res, ResMut, WorldBuilder, fan_out};

    // =========================================================================
    // Core dispatch
    // =========================================================================

    #[test]
    fn step_pure_transform() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().then(|x: u32| x as u64 * 2, r);
        assert_eq!(p.run(&mut world, 5), 10u64);
    }

    #[test]
    fn step_one_res() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        let mut world = wb.build();

        fn multiply(factor: Res<u64>, x: u32) -> u64 {
            *factor * x as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().then(multiply, r);
        assert_eq!(p.run(&mut world, 5), 50);
    }

    #[test]
    fn step_one_res_mut() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut total: ResMut<u64>, x: u32) {
            *total += x as u64;
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().then(accumulate, r);
        p.run(&mut world, 10);
        p.run(&mut world, 5);
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn step_two_params() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        wb.register::<bool>(true);
        let mut world = wb.build();

        fn conditional(factor: Res<u64>, flag: Res<bool>, x: u32) -> u64 {
            if *flag { *factor * x as u64 } else { 0 }
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().then(conditional, r);
        assert_eq!(p.run(&mut world, 5), 50);
    }

    #[test]
    fn step_chain_two() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(2);
        let mut world = wb.build();

        fn double(factor: Res<u64>, x: u32) -> u64 {
            *factor * x as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(double, r)
            .then(|val: u64| val + 1, r);
        assert_eq!(p.run(&mut world, 5), 11); // 2*5 + 1
    }

    // =========================================================================
    // Option combinators
    // =========================================================================

    #[test]
    fn option_map_on_some() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        let mut world = wb.build();

        fn add_factor(factor: Res<u64>, x: u32) -> u64 {
            *factor + x as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u32> { Some(x) }, r)
            .map(add_factor, r);
        assert_eq!(p.run(&mut world, 5), Some(15));
    }

    #[test]
    fn option_map_skips_none() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        fn mark(mut flag: ResMut<bool>, _x: u32) -> u32 {
            *flag = true;
            0
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Option<u32> { None }, r)
            .map(mark, r);
        assert_eq!(p.run(&mut world, 5), None);
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn option_and_then_chains() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        let mut world = wb.build();

        fn check(min: Res<u64>, x: u32) -> Option<u64> {
            let val = x as u64;
            if val > *min { Some(val) } else { None }
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| Some(x), r)
            .and_then(check, r);
        assert_eq!(p.run(&mut world, 20), Some(20));
    }

    #[test]
    fn option_and_then_short_circuits() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        let mut world = wb.build();

        fn check(min: Res<u64>, x: u32) -> Option<u64> {
            let val = x as u64;
            if val > *min { Some(val) } else { None }
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| Some(x), r)
            .and_then(check, r);
        assert_eq!(p.run(&mut world, 5), None);
    }

    #[test]
    fn option_on_none_fires() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let r = world.registry();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Option<u32> { None }, r)
            .on_none(|w: &mut World| {
                *w.resource_mut::<bool>() = true;
            }, r);
        p.run(&mut world, 0);
        assert!(*world.resource::<bool>());
    }

    #[test]
    fn option_filter_keeps() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| Some(x), r)
            .filter(|x: &u32| *x > 3, r);
        assert_eq!(p.run(&mut world, 5), Some(5));
    }

    #[test]
    fn option_filter_drops() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| Some(x), r)
            .filter(|x: &u32| *x > 10, r);
        assert_eq!(p.run(&mut world, 5), None);
    }

    // =========================================================================
    // Result combinators
    // =========================================================================

    #[test]
    fn result_map_on_ok() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        let mut world = wb.build();

        fn add_factor(factor: Res<u64>, x: u32) -> u64 {
            *factor + x as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Result<u32, String> { Ok(x) }, r)
            .map(add_factor, r);
        assert_eq!(p.run(&mut world, 5), Ok(15));
    }

    #[test]
    fn result_map_skips_err() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        fn mark(mut flag: ResMut<bool>, _x: u32) -> u32 {
            *flag = true;
            0
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Result<u32, String> { Err("fail".into()) }, r)
            .map(mark, r);
        assert!(p.run(&mut world, 5).is_err());
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn result_catch_handles_error() {
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();

        fn log_error(mut log: ResMut<String>, err: String) {
            *log = err;
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Result<u32, String> { Err("caught".into()) }, r)
            .catch(log_error, r);
        assert_eq!(p.run(&mut world, 0), None);
        assert_eq!(world.resource::<String>().as_str(), "caught");
    }

    #[test]
    fn result_catch_passes_ok() {
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();

        fn log_error(mut log: ResMut<String>, err: String) {
            *log = err;
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Result<u32, String> { Ok(x) }, r)
            .catch(log_error, r);
        assert_eq!(p.run(&mut world, 5), Some(5));
        assert!(world.resource::<String>().is_empty());
    }

    // =========================================================================
    // Build + Handler
    // =========================================================================

    #[test]
    fn build_produces_handler() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut total: ResMut<u64>, x: u32) {
            *total += x as u64;
        }

        let r = world.registry_mut();
        let mut pipeline = PipelineStart::<u32>::new().then(accumulate, r).build();

        pipeline.run(&mut world, 10);
        pipeline.run(&mut world, 5);
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn run_returns_output() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(3);
        let mut world = wb.build();

        fn multiply(factor: Res<u64>, x: u32) -> u64 {
            *factor * x as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().then(multiply, r);
        let result: u64 = p.run(&mut world, 7);
        assert_eq!(result, 21);
    }

    // =========================================================================
    // Safety
    // =========================================================================

    #[test]
    #[should_panic(expected = "not registered")]
    fn panics_on_missing_resource() {
        let mut world = WorldBuilder::new().build();

        fn needs_u64(_val: Res<u64>, _x: u32) -> u32 {
            0
        }

        let r = world.registry_mut();
        let _p = PipelineStart::<u32>::new().then(needs_u64, r);
    }

    // =========================================================================
    // Access conflict detection
    // =========================================================================

    #[test]
    #[should_panic(expected = "conflicting access")]
    fn step_duplicate_access_panics() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn bad(a: Res<u64>, b: ResMut<u64>, _x: u32) -> u32 {
            let _ = (*a, &*b);
            0
        }

        let r = world.registry_mut();
        let _p = PipelineStart::<u32>::new().then(bad, r);
    }

    // =========================================================================
    // Integration
    // =========================================================================

    #[test]
    fn local_in_step() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn count(mut count: Local<u64>, mut total: ResMut<u64>, _x: u32) {
            *count += 1;
            *total = *count;
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().then(count, r);
        p.run(&mut world, 0);
        p.run(&mut world, 0);
        p.run(&mut world, 0);
        assert_eq!(*world.resource::<u64>(), 3);
    }

    // =========================================================================
    // Option combinators (extended)
    // =========================================================================

    #[test]
    fn option_unwrap_or_some() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u32> { Some(x) }, r)
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 5), 5);
    }

    #[test]
    fn option_unwrap_or_none() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Option<u32> { None }, r)
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 5), 99);
    }

    #[test]
    fn option_unwrap_or_else() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Option<u32> { None }, r)
            .unwrap_or_else(|| 42, r);
        assert_eq!(p.run(&mut world, 0), 42);
    }

    #[test]
    fn option_ok_or() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Option<u32> { None }, r)
            .ok_or("missing");
        assert_eq!(p.run(&mut world, 0), Err("missing"));
    }

    #[test]
    fn option_ok_or_some() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u32> { Some(x) }, r)
            .ok_or("missing");
        assert_eq!(p.run(&mut world, 7), Ok(7));
    }

    #[test]
    fn option_ok_or_else() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Option<u32> { None }, r)
            .ok_or_else(|| "computed", r);
        assert_eq!(p.run(&mut world, 0), Err("computed"));
    }

    #[test]
    fn option_inspect_passes_through() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u32> { Some(x) }, r)
            .inspect(|_val: &u32| {}, r);
        // inspect should pass through the value unchanged.
        assert_eq!(p.run(&mut world, 10), Some(10));
    }

    // =========================================================================
    // Result combinators (extended)
    // =========================================================================

    #[test]
    fn result_map_err() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Result<u32, i32> { Err(-1) }, r)
            .map_err(|e: i32| e.to_string(), r);
        assert_eq!(p.run(&mut world, 0), Err("-1".to_string()));
    }

    #[test]
    fn result_map_err_ok_passthrough() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Result<u32, i32> { Ok(x) }, r)
            .map_err(|e: i32| e.to_string(), r);
        assert_eq!(p.run(&mut world, 5), Ok(5));
    }

    #[test]
    fn result_or_else() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Result<u32, &str> { Err("fail") }, r)
            .or_else(|_e: &str| Ok::<u32, &str>(42), r);
        assert_eq!(p.run(&mut world, 0), Ok(42));
    }

    #[test]
    fn result_inspect_passes_through() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Result<u32, &str> { Ok(x) }, r)
            .inspect(|_val: &u32| {}, r);
        // inspect should pass through Ok unchanged.
        assert_eq!(p.run(&mut world, 7), Ok(7));
    }

    #[test]
    fn result_inspect_err_passes_through() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Result<u32, &str> { Err("bad") }, r)
            .inspect_err(|_e: &&str| {}, r);
        // inspect_err should pass through Err unchanged.
        assert_eq!(p.run(&mut world, 0), Err("bad"));
    }

    #[test]
    fn result_ok_converts() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Result<u32, &str> { Ok(x) }, r)
            .ok();
        assert_eq!(p.run(&mut world, 5), Some(5));
    }

    #[test]
    fn result_ok_drops_err() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Result<u32, &str> { Err("gone") }, r)
            .ok();
        assert_eq!(p.run(&mut world, 0), None);
    }

    #[test]
    fn result_unwrap_or() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Result<u32, &str> { Err("x") }, r)
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 0), 99);
    }

    #[test]
    fn result_unwrap_or_else() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| -> Result<u32, i32> { Err(-5) }, r)
            .unwrap_or_else(|e: i32| e.unsigned_abs(), r);
        assert_eq!(p.run(&mut world, 0), 5);
    }

    // =========================================================================
    // Batch pipeline
    // =========================================================================

    #[test]
    fn batch_accumulates() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut sum: ResMut<u64>, x: u32) {
            *sum += x as u64;
        }

        let r = world.registry_mut();
        let mut batch = PipelineStart::<u32>::new()
            .then(accumulate, r)
            .build_batch(16);

        batch.input_mut().extend_from_slice(&[1, 2, 3, 4, 5]);
        batch.run(&mut world);

        assert_eq!(*world.resource::<u64>(), 15);
        assert!(batch.input().is_empty());
    }

    #[test]
    fn batch_retains_allocation() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut batch = PipelineStart::<u32>::new()
            .then(|_x: u32| {}, r)
            .build_batch(64);

        batch.input_mut().extend_from_slice(&[1, 2, 3]);
        batch.run(&mut world);

        assert!(batch.input().is_empty());
        assert!(batch.input_mut().capacity() >= 64);
    }

    #[test]
    fn batch_empty_is_noop() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut sum: ResMut<u64>, x: u32) {
            *sum += x as u64;
        }

        let r = world.registry_mut();
        let mut batch = PipelineStart::<u32>::new()
            .then(accumulate, r)
            .build_batch(16);

        batch.run(&mut world);
        assert_eq!(*world.resource::<u64>(), 0);
    }

    #[test]
    fn batch_catch_continues_on_error() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<u32>(0);
        let mut world = wb.build();

        fn validate(x: u32) -> Result<u32, &'static str> {
            if x > 0 { Ok(x) } else { Err("zero") }
        }

        fn count_errors(mut errs: ResMut<u32>, _err: &'static str) {
            *errs += 1;
        }

        fn accumulate(mut sum: ResMut<u64>, x: u32) {
            *sum += x as u64;
        }

        let r = world.registry_mut();
        let mut batch = PipelineStart::<u32>::new()
            .then(validate, r)
            .catch(count_errors, r)
            .map(accumulate, r)
            .build_batch(16);

        // Items: 1, 0 (error), 2, 0 (error), 3
        batch.input_mut().extend_from_slice(&[1, 0, 2, 0, 3]);
        batch.run(&mut world);

        assert_eq!(*world.resource::<u64>(), 6); // 1 + 2 + 3
        assert_eq!(*world.resource::<u32>(), 2); // 2 errors
    }

    #[test]
    fn batch_filter_skips_items() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut sum: ResMut<u64>, x: u32) {
            *sum += x as u64;
        }

        let r = world.registry_mut();
        let mut batch = PipelineStart::<u32>::new()
            .then(
                |x: u32| -> Option<u32> { if x > 2 { Some(x) } else { None } },
                r,
            )
            .map(accumulate, r)
            .build_batch(16);

        batch.input_mut().extend_from_slice(&[1, 2, 3, 4, 5]);
        batch.run(&mut world);

        assert_eq!(*world.resource::<u64>(), 12); // 3 + 4 + 5
    }

    #[test]
    fn batch_multiple_runs_accumulate() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut sum: ResMut<u64>, x: u32) {
            *sum += x as u64;
        }

        let r = world.registry_mut();
        let mut batch = PipelineStart::<u32>::new()
            .then(accumulate, r)
            .build_batch(16);

        batch.input_mut().extend_from_slice(&[1, 2, 3]);
        batch.run(&mut world);
        assert_eq!(*world.resource::<u64>(), 6);

        batch.input_mut().extend_from_slice(&[4, 5]);
        batch.run(&mut world);
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn batch_with_world_access() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10); // multiplier
        wb.register::<Vec<u64>>(Vec::new());
        let mut world = wb.build();

        fn multiply_and_collect(factor: Res<u64>, mut out: ResMut<Vec<u64>>, x: u32) {
            out.push(x as u64 * *factor);
        }

        let r = world.registry_mut();
        let mut batch = PipelineStart::<u32>::new()
            .then(multiply_and_collect, r)
            .build_batch(16);

        batch.input_mut().extend_from_slice(&[1, 2, 3]);
        batch.run(&mut world);

        assert_eq!(world.resource::<Vec<u64>>().as_slice(), &[10, 20, 30]);
    }

    // =========================================================================
    // Cloned combinator
    // =========================================================================

    // Named functions for proper lifetime elision (&'a u32 → &'a u32).
    // Closures get two independent lifetimes and fail to compile.
    fn ref_identity(x: &u32) -> &u32 {
        x
    }
    fn ref_wrap_some(x: &u32) -> Option<&u32> {
        Some(x)
    }
    fn ref_wrap_none(_x: &u32) -> Option<&u32> {
        None
    }
    fn ref_wrap_ok(x: &u32) -> Result<&u32, String> {
        Ok(x)
    }
    fn ref_wrap_err(_x: &u32) -> Result<&u32, String> {
        Err("fail".into())
    }

    #[test]
    fn cloned_bare() {
        let mut world = WorldBuilder::new().build();
        // val before p — val must outlive the pipeline's In = &u32
        let val = 42u32;
        let r = world.registry_mut();
        let mut p = PipelineStart::<&u32>::new().then(ref_identity, r).cloned();
        assert_eq!(p.run(&mut world, &val), 42u32);
    }

    #[test]
    fn cloned_option_some() {
        let mut world = WorldBuilder::new().build();
        let val = 42u32;
        let r = world.registry_mut();
        let mut p = PipelineStart::<&u32>::new().then(ref_wrap_some, r).cloned();
        assert_eq!(p.run(&mut world, &val), Some(42u32));
    }

    #[test]
    fn cloned_option_none() {
        let mut world = WorldBuilder::new().build();
        let val = 42u32;
        let r = world.registry_mut();
        let mut p = PipelineStart::<&u32>::new().then(ref_wrap_none, r).cloned();
        assert_eq!(p.run(&mut world, &val), None);
    }

    #[test]
    fn cloned_result_ok() {
        let mut world = WorldBuilder::new().build();
        let val = 42u32;
        let r = world.registry_mut();
        let mut p = PipelineStart::<&u32>::new().then(ref_wrap_ok, r).cloned();
        assert_eq!(p.run(&mut world, &val), Ok(42u32));
    }

    #[test]
    fn cloned_result_err() {
        let mut world = WorldBuilder::new().build();
        let val = 42u32;
        let r = world.registry_mut();
        let mut p = PipelineStart::<&u32>::new().then(ref_wrap_err, r).cloned();
        assert_eq!(p.run(&mut world, &val), Err("fail".into()));
    }

    // =========================================================================
    // Dispatch combinator
    // =========================================================================

    #[test]
    fn dispatch_to_handler() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn store(mut out: ResMut<u64>, val: u32) {
            *out = val as u64;
        }

        let r = world.registry_mut();
        let handler = PipelineStart::<u32>::new().then(store, r).build();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x * 2, r)
            .dispatch(handler)
            .build();

        p.run(&mut world, 5);
        assert_eq!(*world.resource::<u64>(), 10);
    }

    #[test]
    fn dispatch_to_fanout() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<i64>(0);
        let mut world = wb.build();

        fn write_u64(mut sink: ResMut<u64>, event: &u32) {
            *sink += *event as u64;
        }
        fn write_i64(mut sink: ResMut<i64>, event: &u32) {
            *sink += *event as i64;
        }

        let h1 = write_u64.into_handler(world.registry());
        let h2 = write_i64.into_handler(world.registry());
        let fan = fan_out!(h1, h2);

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x * 2, r)
            .dispatch(fan)
            .build();

        p.run(&mut world, 5);
        assert_eq!(*world.resource::<u64>(), 10);
        assert_eq!(*world.resource::<i64>(), 10);
    }

    #[test]
    fn dispatch_to_broadcast() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn write_u64(mut sink: ResMut<u64>, event: &u32) {
            *sink += *event as u64;
        }

        let mut broadcast = crate::Broadcast::<u32>::new();
        broadcast.add(write_u64.into_handler(world.registry()));
        broadcast.add(write_u64.into_handler(world.registry()));

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x + 1, r)
            .dispatch(broadcast)
            .build();

        p.run(&mut world, 4);
        assert_eq!(*world.resource::<u64>(), 10); // 5 + 5
    }

    #[test]
    fn dispatch_build_produces_handler() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn store(mut out: ResMut<u64>, val: u32) {
            *out = val as u64;
        }

        let r = world.registry_mut();
        let inner = PipelineStart::<u32>::new().then(store, r).build();

        let mut pipeline: Box<dyn Handler<u32>> = Box::new(
            PipelineStart::<u32>::new()
                .then(|x: u32| x + 1, r)
                .dispatch(inner)
                .build(),
        );

        pipeline.run(&mut world, 9);
        assert_eq!(*world.resource::<u64>(), 10);
    }

    // -- Guard combinator --

    #[test]
    fn pipeline_guard_keeps() {
        fn sink(mut out: ResMut<u64>, val: Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64, reg)
            .guard(|v: &u64| *v > 3, reg)
            .then(sink, reg);

        p.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 5);
    }

    #[test]
    fn pipeline_guard_drops() {
        fn sink(mut out: ResMut<u64>, val: Option<u64>) {
            *out = val.unwrap_or(999);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64, reg)
            .guard(|v: &u64| *v > 10, reg)
            .then(sink, reg);

        p.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 999);
    }

    // -- Tap combinator --

    #[test]
    fn pipeline_tap_observes_without_changing() {
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<bool>(false);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64 * 2, reg)
            .tap(|w: &mut World, val: &u64| {
                *w.resource_mut::<bool>() = *val == 10;
            }, reg)
            .then(sink, reg);

        p.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 10); // value passed through
        assert!(*world.resource::<bool>()); // tap fired
    }

    // -- Route combinator --

    #[test]
    fn pipeline_route_true_arm() {
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let arm_t = PipelineStart::new().then(|x: u64| x * 2, reg);
        let arm_f = PipelineStart::new().then(|x: u64| x * 3, reg);

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64, reg)
            .route(|v: &u64| *v > 3, reg, arm_t, arm_f)
            .then(sink, reg);

        p.run(&mut world, 5u32); // 5 > 3 → true arm → double → 10
        assert_eq!(*world.resource::<u64>(), 10);
    }

    #[test]
    fn pipeline_route_false_arm() {
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let arm_t = PipelineStart::new().then(|x: u64| x * 2, reg);
        let arm_f = PipelineStart::new().then(|x: u64| x * 3, reg);

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64, reg)
            .route(|v: &u64| *v > 10, reg, arm_t, arm_f)
            .then(sink, reg);

        p.run(&mut world, 5u32); // 5 <= 10 → false arm → triple → 15
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn pipeline_route_nested() {
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        // N-ary via nesting: <5 → +100, 5..10 → +200, >=10 → +300
        let inner_t = PipelineStart::new().then(|x: u64| x + 200, reg);
        let inner_f = PipelineStart::new().then(|x: u64| x + 300, reg);
        let outer_t = PipelineStart::new().then(|x: u64| x + 100, reg);
        let outer_f =
            PipelineStart::new()
                .then(|x: u64| x, reg)
                .route(|v: &u64| *v < 10, reg, inner_t, inner_f);

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64, reg)
            .route(|v: &u64| *v < 5, reg, outer_t, outer_f)
            .then(sink, reg);

        p.run(&mut world, 3u32); // 3 < 5 → +100 → 103
        assert_eq!(*world.resource::<u64>(), 103);

        p.run(&mut world, 7u32); // 7 >= 5, 7 < 10 → +200 → 207
        assert_eq!(*world.resource::<u64>(), 207);

        p.run(&mut world, 15u32); // 15 >= 5, 15 >= 10 → +300 → 315
        assert_eq!(*world.resource::<u64>(), 315);
    }

    // -- Tee combinator --

    #[test]
    fn pipeline_tee_side_effect_chain() {
        use crate::dag::DagArmStart;

        fn log_step(mut counter: ResMut<u32>, _val: &u64) {
            *counter += 1;
        }
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<u32>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let side = DagArmStart::new().then(log_step, reg);

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64 * 2, reg)
            .tee(side)
            .then(sink, reg);

        p.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 10); // value passed through
        assert_eq!(*world.resource::<u32>(), 1); // side-effect fired

        p.run(&mut world, 7u32);
        assert_eq!(*world.resource::<u64>(), 14);
        assert_eq!(*world.resource::<u32>(), 2);
    }

    // -- Dedup combinator --

    #[test]
    fn pipeline_dedup_suppresses_unchanged() {
        fn sink(mut out: ResMut<u32>, val: Option<u64>) {
            if val.is_some() {
                *out += 1;
            }
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u32>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64 / 2, reg)
            .dedup()
            .then(sink, reg);

        p.run(&mut world, 4u32); // 2 — first, Some
        assert_eq!(*world.resource::<u32>(), 1);

        p.run(&mut world, 5u32); // 2 — same, None
        assert_eq!(*world.resource::<u32>(), 1);

        p.run(&mut world, 6u32); // 3 — changed, Some
        assert_eq!(*world.resource::<u32>(), 2);
    }

    // -- Bool combinators --

    #[test]
    fn pipeline_not() {
        fn sink(mut out: ResMut<bool>, val: bool) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x > 5, reg)
            .not()
            .then(sink, reg);

        p.run(&mut world, 3u32); // 3 > 5 = false, not = true
        assert!(*world.resource::<bool>());

        p.run(&mut world, 10u32); // 10 > 5 = true, not = false
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn pipeline_and() {
        fn sink(mut out: ResMut<bool>, val: bool) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(true);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x > 5, reg)
            .and(|w: &mut World| *w.resource::<bool>(), reg)
            .then(sink, reg);

        p.run(&mut world, 10u32); // true && true = true
        assert!(*world.resource::<bool>());

        *world.resource_mut::<bool>() = false;
        p.run(&mut world, 10u32); // true && false = false
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn pipeline_or() {
        fn sink(mut out: ResMut<bool>, val: bool) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x > 5, reg)
            .or(|w: &mut World| *w.resource::<bool>(), reg)
            .then(sink, reg);

        p.run(&mut world, 3u32); // false || false = false
        assert!(!*world.resource::<bool>());

        *world.resource_mut::<bool>() = true;
        p.run(&mut world, 3u32); // false || true = true
        assert!(*world.resource::<bool>());
    }

    #[test]
    fn pipeline_xor() {
        fn sink(mut out: ResMut<bool>, val: bool) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(true);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x > 5, reg)
            .xor(|w: &mut World| *w.resource::<bool>(), reg)
            .then(sink, reg);

        p.run(&mut world, 10u32); // true ^ true = false
        assert!(!*world.resource::<bool>());
    }

    // =========================================================================
    // Splat — tuple destructuring
    // =========================================================================

    #[test]
    fn splat2_closure_on_start() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<(u32, u64)>::new()
            .splat()
            .then(|a: u32, b: u64| a as u64 + b, r);
        assert_eq!(p.run(&mut world, (3, 7)), 10);
    }

    #[test]
    fn splat2_named_fn_with_param() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(100);
        let mut world = wb.build();

        fn process(base: Res<u64>, a: u32, b: u32) -> u64 {
            *base + a as u64 + b as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<(u32, u32)>::new().splat().then(process, r);
        assert_eq!(p.run(&mut world, (3, 7)), 110);
    }

    #[test]
    fn splat2_mid_chain() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| (x, x * 2), r)
            .splat()
            .then(|a: u32, b: u32| a as u64 + b as u64, r);
        assert_eq!(p.run(&mut world, 5), 15); // 5 + 10
    }

    #[test]
    fn splat3_closure_on_start() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<(u32, u32, u32)>::new()
            .splat()
            .then(|a: u32, b: u32, c: u32| a + b + c, r);
        assert_eq!(p.run(&mut world, (1, 2, 3)), 6);
    }

    #[test]
    fn splat3_named_fn_with_param() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        let mut world = wb.build();

        fn process(factor: Res<u64>, a: u32, b: u32, c: u32) -> u64 {
            *factor * (a + b + c) as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<(u32, u32, u32)>::new()
            .splat()
            .then(process, r);
        assert_eq!(p.run(&mut world, (1, 2, 3)), 60);
    }

    #[test]
    fn splat4_mid_chain() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| (x, x + 1, x + 2, x + 3), r)
            .splat()
            .then(|a: u32, b: u32, c: u32, d: u32| (a + b + c + d) as u64, r);
        assert_eq!(p.run(&mut world, 10), 46); // 10+11+12+13
    }

    #[test]
    fn splat5_closure_on_start() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<(u8, u8, u8, u8, u8)>::new().splat().then(
            |a: u8, b: u8, c: u8, d: u8, e: u8| {
                (a as u64) + (b as u64) + (c as u64) + (d as u64) + (e as u64)
            },
            r,
        );
        assert_eq!(p.run(&mut world, (1, 2, 3, 4, 5)), 15);
    }

    #[test]
    fn splat_build_into_handler() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn store(mut out: ResMut<u64>, a: u32, b: u32) {
            *out = a as u64 + b as u64;
        }

        let r = world.registry_mut();
        let mut pipeline = PipelineStart::<(u32, u32)>::new()
            .splat()
            .then(store, r)
            .build();

        pipeline.run(&mut world, (3, 7));
        assert_eq!(*world.resource::<u64>(), 10);
    }

    #[test]
    fn splat_build_batch() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut sum: ResMut<u64>, a: u32, b: u32) {
            *sum += a as u64 + b as u64;
        }

        let r = world.registry_mut();
        let mut batch = PipelineStart::<(u32, u32)>::new()
            .splat()
            .then(accumulate, r)
            .build_batch(8);

        batch
            .input_mut()
            .extend_from_slice(&[(1, 2), (3, 4), (5, 6)]);
        batch.run(&mut world);
        assert_eq!(*world.resource::<u64>(), 21); // 3+7+11
    }

    #[test]
    #[should_panic]
    fn splat_access_conflict_detected() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn bad(a: ResMut<u64>, _b: ResMut<u64>, _x: u32, _y: u32) {
            let _ = a;
        }

        let r = world.registry_mut();
        // Should panic on duplicate ResMut<u64>
        let _ = PipelineStart::<(u32, u32)>::new().splat().then(bad, r);
    }

    // -- Then (previously switch) --

    #[test]
    fn pipeline_then_branching() {
        fn double(x: u32) -> u64 {
            x as u64 * 2
        }
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }

        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut pipeline = PipelineStart::<u32>::new()
            .then(double, reg)
            .then(|val: u64| if val > 10 { val * 100 } else { val + 1 }, reg)
            .then(sink, reg)
            .build();

        pipeline.run(&mut world, 10u32); // 20 > 10 → 2000
        assert_eq!(*world.resource::<u64>(), 2000);

        pipeline.run(&mut world, 3u32); // 6 <= 10 → 7
        assert_eq!(*world.resource::<u64>(), 7);
    }

    #[test]
    fn pipeline_then_3_way() {
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }

        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut pipeline = PipelineStart::<u32>::new()
            .then(
                |val: u32| match val % 3 {
                    0 => val as u64 + 100,
                    1 => val as u64 + 200,
                    _ => val as u64 + 300,
                },
                reg,
            )
            .then(sink, reg)
            .build();

        pipeline.run(&mut world, 6u32); // 6 % 3 == 0 → 106
        assert_eq!(*world.resource::<u64>(), 106);

        pipeline.run(&mut world, 7u32); // 7 % 3 == 1 → 207
        assert_eq!(*world.resource::<u64>(), 207);

        pipeline.run(&mut world, 8u32); // 8 % 3 == 2 → 308
        assert_eq!(*world.resource::<u64>(), 308);
    }

    #[test]
    fn pipeline_then_with_resolve_step() {
        fn add_offset(offset: Res<i64>, val: u32) -> u64 {
            (*offset + val as i64) as u64
        }
        fn plain_double(val: u32) -> u64 {
            val as u64 * 2
        }
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }

        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<i64>(100);
        let mut world = wb.build();
        let reg = world.registry();

        let mut arm_offset = resolve_step(add_offset, reg);
        let mut arm_double = resolve_step(plain_double, reg);

        let mut pipeline = PipelineStart::<u32>::new()
            .then(
                move |world: &mut World, val: u32| {
                    if val > 10 {
                        arm_offset(world, val)
                    } else {
                        arm_double(world, val)
                    }
                },
                reg,
            )
            .then(sink, reg)
            .build();

        pipeline.run(&mut world, 20u32); // > 10 → offset → 100 + 20 = 120
        assert_eq!(*world.resource::<u64>(), 120);

        pipeline.run(&mut world, 5u32); // <= 10 → double → 10
        assert_eq!(*world.resource::<u64>(), 10);
    }

    #[test]
    fn batch_pipeline_then_branching() {
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out += val;
        }

        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut batch = PipelineStart::<u32>::new()
            .then(
                |val: u32| {
                    if val % 2 == 0 {
                        val as u64 * 10
                    } else {
                        val as u64
                    }
                },
                reg,
            )
            .then(sink, reg)
            .build_batch(8);

        batch.input_mut().extend([1, 2, 3, 4]);
        batch.run(&mut world);

        // 1 → 1, 2 → 20, 3 → 3, 4 → 40 = 64
        assert_eq!(*world.resource::<u64>(), 64);
    }

    // -- IntoRefStep with Param: named functions --

    #[test]
    fn guard_named_fn_with_param() {
        fn above_threshold(threshold: Res<u64>, val: &u64) -> bool {
            *val > *threshold
        }
        fn sink(mut out: ResMut<i64>, val: Option<u64>) {
            *out = val.map(|v| v as i64).unwrap_or(-1);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(5); // threshold
        wb.register::<i64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64, reg)
            .guard(above_threshold, reg)
            .then(sink, reg);

        p.run(&mut world, 10u32); // 10 > 5 → Some(10)
        assert_eq!(*world.resource::<i64>(), 10);

        p.run(&mut world, 3u32); // 3 <= 5 → None → -1
        assert_eq!(*world.resource::<i64>(), -1);
    }

    #[test]
    fn filter_named_fn_with_param() {
        fn is_allowed(allowed: Res<u64>, val: &u64) -> bool {
            *val != *allowed
        }
        fn count(mut ctr: ResMut<i64>, _val: u64) {
            *ctr += 1;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(42); // blocked value
        wb.register::<i64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u64> { Some(x as u64) }, reg)
            .filter(is_allowed, reg)
            .map(count, reg)
            .unwrap_or(());

        for v in [1u32, 42, 5, 42, 10] {
            p.run(&mut world, v);
        }
        assert_eq!(*world.resource::<i64>(), 3); // 42 filtered out twice
    }

    #[test]
    fn inspect_named_fn_with_param() {
        fn log_value(mut log: ResMut<Vec<u64>>, val: &u64) {
            log.push(*val);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<Vec<u64>>(Vec::new());
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u64> { Some(x as u64) }, reg)
            .inspect(log_value, reg)
            .unwrap_or(0);

        for v in [1u32, 2, 3] {
            p.run(&mut world, v);
        }
        assert_eq!(world.resource::<Vec<u64>>().as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn tap_named_fn_with_param() {
        fn observe(mut log: ResMut<Vec<u64>>, val: &u64) {
            log.push(*val);
        }
        fn sink(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<Vec<u64>>(Vec::new());
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| x as u64, reg)
            .tap(observe, reg)
            .then(sink, reg);

        p.run(&mut world, 7u32);
        assert_eq!(*world.resource::<u64>(), 7);
        assert_eq!(world.resource::<Vec<u64>>().as_slice(), &[7]);
    }

    // -- IntoProducer with Param: named functions --

    #[test]
    fn and_named_fn_with_param() {
        fn check_enabled(flag: Res<bool>) -> bool {
            *flag
        }
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(true);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| true, reg)
            .and(check_enabled, reg);

        assert!(p.run(&mut world, 0u32));

        *world.resource_mut::<bool>() = false;
        assert!(!p.run(&mut world, 0u32)); // short-circuit: true AND false
    }

    #[test]
    fn or_named_fn_with_param() {
        fn check_enabled(flag: Res<bool>) -> bool {
            *flag
        }
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(true);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|_x: u32| false, reg)
            .or(check_enabled, reg);

        assert!(p.run(&mut world, 0u32)); // false OR true

        *world.resource_mut::<bool>() = false;
        assert!(!p.run(&mut world, 0u32)); // false OR false
    }

    #[test]
    fn on_none_named_fn_with_param() {
        fn log_miss(mut ctr: ResMut<u64>) {
            *ctr += 1;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u32> { if x > 5 { Some(x) } else { None } }, reg)
            .on_none(log_miss, reg)
            .unwrap_or(0);

        for v in [1u32, 10, 3, 20] {
            p.run(&mut world, v);
        }
        assert_eq!(*world.resource::<u64>(), 2); // 1 and 3 are None
    }

    #[test]
    fn ok_or_else_named_fn_with_param() {
        fn make_error(msg: Res<String>) -> String {
            msg.clone()
        }
        let mut wb = WorldBuilder::new();
        wb.register::<String>("not found".into());
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u32> { if x > 0 { Some(x) } else { None } }, reg)
            .ok_or_else(make_error, reg);

        let r: Result<u32, String> = p.run(&mut world, 5u32);
        assert_eq!(r, Ok(5));

        let r: Result<u32, String> = p.run(&mut world, 0u32);
        assert_eq!(r, Err("not found".into()));
    }

    #[test]
    fn unwrap_or_else_option_named_fn_with_param() {
        fn fallback(default: Res<u64>) -> u64 {
            *default
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(42);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Option<u64> { if x > 0 { Some(x as u64) } else { None } }, reg)
            .unwrap_or_else(fallback, reg);

        assert_eq!(p.run(&mut world, 5u32), 5);
        assert_eq!(p.run(&mut world, 0u32), 42);
    }

    // -- IntoStep with Opaque: &mut World closures --

    #[test]
    fn map_err_named_fn_with_param() {
        fn tag_error(prefix: Res<String>, err: String) -> String {
            format!("{}: {err}", &*prefix)
        }
        fn sink(mut out: ResMut<String>, val: Result<u32, String>) {
            match val {
                Ok(v) => *out = format!("ok:{v}"),
                Err(e) => *out = e,
            }
        }
        let mut wb = WorldBuilder::new();
        wb.register::<String>("ERR".into());
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u32>::new()
            .then(|x: u32| -> Result<u32, String> {
                if x > 0 { Ok(x) } else { Err("zero".into()) }
            }, reg)
            .map_err(tag_error, reg)
            .then(sink, reg);

        p.run(&mut world, 0u32);
        assert_eq!(world.resource::<String>().as_str(), "ERR: zero");

        p.run(&mut world, 5u32);
        assert_eq!(world.resource::<String>().as_str(), "ok:5");
    }

    // =========================================================================
    // Scan combinator
    // =========================================================================

    #[test]
    fn scan_arity0_closure_running_sum() {
        let mut world = WorldBuilder::new().build();
        let reg = world.registry();

        let mut p = PipelineStart::<u64>::new()
            .then(|x: u64| x, reg)
            .scan(0u64, |acc: &mut u64, val: u64| {
                *acc += val;
                Some(*acc)
            }, reg);

        assert_eq!(p.run(&mut world, 10), Some(10));
        assert_eq!(p.run(&mut world, 20), Some(30));
        assert_eq!(p.run(&mut world, 5), Some(35));
    }

    #[test]
    fn scan_named_fn_with_param() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(100);
        let mut world = wb.build();
        let reg = world.registry();

        fn threshold_scan(
            limit: Res<u64>,
            acc: &mut u64,
            val: u64,
        ) -> Option<u64> {
            *acc += val;
            if *acc > *limit { Some(*acc) } else { None }
        }

        let mut p = PipelineStart::<u64>::new()
            .then(|x: u64| x, reg)
            .scan(0u64, threshold_scan, reg);

        assert_eq!(p.run(&mut world, 50), None);
        assert_eq!(p.run(&mut world, 30), None);
        assert_eq!(p.run(&mut world, 25), Some(105));
    }

    #[test]
    fn scan_opaque_closure() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineStart::<u64>::new()
            .then(|x: u64| x, reg)
            .scan(0u64, |world: &mut World, acc: &mut u64, val: u64| {
                let factor = *world.resource::<u64>();
                *acc += val * factor;
                Some(*acc)
            }, reg);

        assert_eq!(p.run(&mut world, 1), Some(10));
        assert_eq!(p.run(&mut world, 2), Some(30));
    }

    #[test]
    fn scan_suppression_returns_none() {
        let mut world = WorldBuilder::new().build();
        let reg = world.registry();

        let mut p = PipelineStart::<u64>::new()
            .then(|x: u64| x, reg)
            .scan(0u64, |acc: &mut u64, val: u64| -> Option<u64> {
                *acc += val;
                if *acc > 50 { Some(*acc) } else { None }
            }, reg);

        assert_eq!(p.run(&mut world, 20), None);
        assert_eq!(p.run(&mut world, 20), None);
        assert_eq!(p.run(&mut world, 20), Some(60));
    }

    #[test]
    fn scan_on_pipeline_start() {
        let mut world = WorldBuilder::new().build();
        let reg = world.registry();

        let mut p = PipelineStart::<u64>::new()
            .scan(0u64, |acc: &mut u64, val: u64| {
                *acc += val;
                *acc
            }, reg);

        assert_eq!(p.run(&mut world, 5), 5);
        assert_eq!(p.run(&mut world, 3), 8);
        assert_eq!(p.run(&mut world, 2), 10);
    }

    #[test]
    fn scan_persistence_across_batch() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn store(mut out: ResMut<u64>, val: u64) { *out = val; }

        let mut p = PipelineStart::<u64>::new()
            .then(|x: u64| x, reg)
            .scan(0u64, |acc: &mut u64, val: u64| {
                *acc += val;
                *acc
            }, reg)
            .then(store, reg)
            .build_batch(4);

        p.input_mut().extend([1, 2, 3]);
        p.run(&mut world);

        // Accumulator persists: 1, 3, 6
        assert_eq!(*world.resource::<u64>(), 6);

        p.input_mut().push(4);
        p.run(&mut world);
        // acc = 6 + 4 = 10
        assert_eq!(*world.resource::<u64>(), 10);
    }

    // =========================================================================
    // Build — Option<()> terminal
    // =========================================================================

    #[test]
    fn build_option_unit_terminal() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let r = world.registry_mut();

        fn check(x: u32) -> Option<u32> {
            if x > 5 { Some(x) } else { None }
        }
        fn store(mut out: ResMut<u64>, val: u32) {
            *out += val as u64;
        }

        // .map(store) on Option<u32> produces Option<()> — build() must work
        let mut p = PipelineStart::<u32>::new()
            .then(check, r)
            .map(store, r)
            .build();

        p.run(&mut world, 3); // None, skipped
        assert_eq!(*world.resource::<u64>(), 0);
        p.run(&mut world, 7); // Some, stores
        assert_eq!(*world.resource::<u64>(), 7);
        p.run(&mut world, 10);
        assert_eq!(*world.resource::<u64>(), 17);
    }

    #[test]
    fn build_option_unit_boxes_into_handler() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let r = world.registry_mut();

        fn double(x: u32) -> Option<u64> {
            if x > 0 { Some(x as u64 * 2) } else { None }
        }
        fn store(mut out: ResMut<u64>, val: u64) {
            *out += val;
        }

        let mut h: Box<dyn Handler<u32>> = Box::new(
            PipelineStart::<u32>::new()
                .then(double, r)
                .map(store, r)
                .build(),
        );
        h.run(&mut world, 0); // None
        assert_eq!(*world.resource::<u64>(), 0);
        h.run(&mut world, 5); // 10
        assert_eq!(*world.resource::<u64>(), 10);
    }

    // =========================================================================
    // Build — borrowed event type
    // =========================================================================

    #[test]
    fn build_borrowed_event_direct() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn decode(msg: &[u8]) -> u64 {
            msg.len() as u64
        }
        fn store(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }

        // msg declared before p so it outlives the pipeline (drop order).
        // Matches real-world usage: pipeline lives long, events come and go.
        let msg = vec![1u8, 2, 3];
        let r = world.registry_mut();
        let mut p = PipelineStart::<&[u8]>::new()
            .then(decode, r)
            .then(store, r)
            .build();

        p.run(&mut world, &msg);
        assert_eq!(*world.resource::<u64>(), 3);
    }

    #[test]
    fn build_borrowed_event_option_unit() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn decode(msg: &[u8]) -> Option<u64> {
            if msg.is_empty() { None } else { Some(msg.len() as u64) }
        }
        fn store(mut out: ResMut<u64>, val: u64) {
            *out = val;
        }

        let empty = vec![];
        let data = vec![1u8, 2, 3];
        let r = world.registry_mut();
        let mut p = PipelineStart::<&[u8]>::new()
            .then(decode, r)
            .map(store, r)
            .build();

        p.run(&mut world, &empty); // None
        assert_eq!(*world.resource::<u64>(), 0);
        p.run(&mut world, &data); // Some(3)
        assert_eq!(*world.resource::<u64>(), 3);
    }
}
