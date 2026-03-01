// Builder return types are necessarily complex — each combinator returns
// PipelineBuilder<In, Out, impl FnMut(...)>. Same pattern as iterator adapters.
#![allow(clippy::type_complexity)]

//! Pre-resolved pipeline dispatch using [`SystemParam`] stages.
//!
//! [`PipelineStart`] begins a typed composition chain where each stage
//! is a named function with [`SystemParam`] dependencies resolved at build
//! time. The result is a monomorphized closure chain where dispatch-time
//! resource access is ~3 cycles per fetch (pre-resolved [`ResourceId`]),
//! not a HashMap lookup.
//!
//! Two dispatch tiers in nexus-rt:
//! 1. **Pipeline** — static after build, pre-resolved, the workhorse
//! 2. **Callback** — dynamic registration with per-instance context
//!
//! # Stage function convention
//!
//! SystemParams first, stage input last, returns output:
//!
//! ```ignore
//! fn validate(config: Res<Config>, order: Order) -> Option<ValidOrder> { .. }
//! fn enrich(cache: Res<MarketData>, order: ValidOrder) -> EnrichedOrder { .. }
//! fn submit(mut gw: ResMut<Gateway>, order: CheckedOrder) { gw.send(order); }
//! ```
//!
//! # Combinator split
//!
//! **IntoStage-based (pre-resolved, hot path):**
//! `.stage()`, `.map()`, `.and_then()`, `.catch()`
//!
//! **Closure-based (cold path, `&mut World`):**
//! `.on_none()`, `.inspect()`, `.inspect_err()`, `.filter()`, `.ok()`,
//! `.unwrap_or()`, `.unwrap_or_else()`, `.map_err()`, `.or_else()`

use std::marker::PhantomData;

use crate::system::SystemParam;
use crate::world::{Registry, World};

// =============================================================================
// Stage — pre-resolved stage with SystemParam state
// =============================================================================

/// Internal: pre-resolved stage with cached SystemParam state.
///
/// Users don't construct this directly — it's produced by [`IntoStage`] and
/// captured inside pipeline chain closures.
#[doc(hidden)]
pub struct Stage<F, Params: SystemParam> {
    f: F,
    state: Params::State,
    #[allow(dead_code)]
    name: &'static str,
}

// =============================================================================
// StageCall — callable trait for resolved stages
// =============================================================================

/// Internal: callable trait for resolved stages.
///
/// Used as a bound on [`IntoStage::Stage`]. Users don't implement this.
#[doc(hidden)]
pub trait StageCall<In, Out> {
    /// Call this stage with a world reference and input value.
    fn call(&mut self, world: &mut World, input: In) -> Out;
}

// =============================================================================
// IntoStage — converts a named function into a resolved stage
// =============================================================================

/// Converts a named function into a pre-resolved pipeline stage.
///
/// SystemParams first, stage input last, returns output. Arity 0 (no
/// SystemParams) supports closures. Arities 1+ require named functions
/// (same HRTB+GAT limitation as [`IntoSystem`](crate::IntoSystem)).
///
/// # Examples
///
/// ```ignore
/// // Arity 0 — closure works
/// let stage = (|x: u32| x * 2).into_stage(registry);
///
/// // Arity 1 — named function required
/// fn validate(config: Res<Config>, order: Order) -> Option<ValidOrder> { .. }
/// let stage = validate.into_stage(registry);
/// ```
pub trait IntoStage<In, Out, Params> {
    /// The concrete resolved stage type.
    type Stage: StageCall<In, Out>;

    /// Resolve SystemParam state from the registry and produce a stage.
    fn into_stage(self, registry: &mut Registry) -> Self::Stage;
}

// =============================================================================
// Arity 0 — fn(In) -> Out — closures work (no HRTB+GAT issues)
// =============================================================================

impl<In, Out, F: FnMut(In) -> Out + 'static> StageCall<In, Out> for Stage<F, ()> {
    #[inline(always)]
    fn call(&mut self, _world: &mut World, input: In) -> Out {
        (self.f)(input)
    }
}

impl<In, Out, F: FnMut(In) -> Out + 'static> IntoStage<In, Out, ()> for F {
    type Stage = Stage<F, ()>;

    fn into_stage(self, registry: &mut Registry) -> Self::Stage {
        Stage {
            f: self,
            state: <() as SystemParam>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

// =============================================================================
// Arities 1-8 via macro — HRTB with -> Out
// =============================================================================

macro_rules! impl_into_stage {
    ($($P:ident),+) => {
        impl<In, Out, F: 'static, $($P: SystemParam + 'static),+>
            StageCall<In, Out> for Stage<F, ($($P,)+)>
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
                let ($($P,)+) = unsafe {
                    <($($P,)+) as SystemParam>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ input)
            }
        }

        impl<In, Out, F: 'static, $($P: SystemParam + 'static),+>
            IntoStage<In, Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ In) -> Out +
                FnMut($($P::Item<'a>,)+ In) -> Out,
        {
            type Stage = Stage<F, ($($P,)+)>;

            fn into_stage(self, registry: &mut Registry) -> Self::Stage {
                let state = <($($P,)+) as SystemParam>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as SystemParam>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                Stage { f: self, state, name: std::any::type_name::<F>() }
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

all_tuples!(impl_into_stage);

// =============================================================================
// PipelineStart — entry point
// =============================================================================

/// Entry point for building a pre-resolved stage pipeline.
///
/// `In` is the pipeline input type. Call [`.stage()`](Self::stage) to add
/// the first stage — a named function whose [`SystemParam`] dependencies
/// are resolved from the registry at build time.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, Res, ResMut, PipelineStart, System};
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
///     .stage(double, r)
///     .stage(store, r)
///     .build();
///
/// pipeline.run(&mut world, 5);
/// assert_eq!(world.resource::<String>().as_str(), "50");
/// ```
pub struct PipelineStart<In>(PhantomData<fn(In)>);

impl<In> PipelineStart<In> {
    /// Create a new stage pipeline entry point.
    pub fn new() -> Self {
        Self(PhantomData)
    }

    /// Add the first stage. SystemParams resolved from the registry.
    pub fn stage<Out, Params, S: IntoStage<In, Out, Params>>(
        self,
        f: S,
        registry: &mut Registry,
    ) -> PipelineBuilder<In, Out, impl FnMut(&mut World, In) -> Out + use<In, Out, Params, S>> {
        let mut resolved = f.into_stage(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| resolved.call(world, input),
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

/// Builder that composes pre-resolved pipeline stages via closure nesting.
///
/// `In` is the pipeline's input type (fixed). `Out` is the current output.
/// `Chain` is the concrete composed closure type (opaque, never named by users).
///
/// Each combinator consumes `self`, captures the previous chain in a new
/// closure, and returns a new `PipelineBuilder`. The compiler
/// monomorphizes the entire chain — zero virtual dispatch through stages.
///
/// IntoStage-based methods (`.stage()`, `.map()`, `.and_then()`, `.catch()`)
/// take `&Registry` to resolve SystemParam state at build time. Closure-based
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
    /// Add a stage. SystemParams resolved from the registry.
    pub fn stage<NewOut, Params, S: IntoStage<Out, NewOut, Params>>(
        self,
        f: S,
        registry: &mut Registry,
    ) -> PipelineBuilder<
        In,
        NewOut,
        impl FnMut(&mut World, In) -> NewOut + use<In, Out, NewOut, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_stage(registry);
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
}

// =============================================================================
// Option helpers — PipelineBuilder<In, Option<T>, Chain>
// =============================================================================

impl<In, T, Chain> PipelineBuilder<In, Option<T>, Chain>
where
    Chain: FnMut(&mut World, In) -> Option<T>,
{
    // -- IntoStage-based (hot path) -------------------------------------------

    /// Transform the inner value. Stage not called on None.
    pub fn map<U, Params, S: IntoStage<T, U, Params>>(
        self,
        f: S,
        registry: &mut Registry,
    ) -> PipelineBuilder<
        In,
        Option<U>,
        impl FnMut(&mut World, In) -> Option<U> + use<In, T, U, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_stage(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).map(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Short-circuits on None. std: `Option::and_then`
    pub fn and_then<U, Params, S: IntoStage<T, Option<U>, Params>>(
        self,
        f: S,
        registry: &mut Registry,
    ) -> PipelineBuilder<
        In,
        Option<U>,
        impl FnMut(&mut World, In) -> Option<U> + use<In, T, U, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_stage(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).and_then(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    // -- Closure-based (cold path, &mut World) --------------------------------

    /// Side effect on None. Complement to [`inspect`](Self::inspect).
    pub fn on_none(
        self,
        mut f: impl FnMut(&mut World) + 'static,
    ) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let result = chain(world, input);
                if result.is_none() {
                    f(world);
                }
                result
            },
            _marker: PhantomData,
        }
    }

    /// Keep value if predicate holds. std: `Option::filter`
    pub fn filter(
        self,
        mut f: impl FnMut(&mut World, &T) -> bool + 'static,
    ) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).filter(|val| f(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Side effect on Some value. std: `Option::inspect`
    pub fn inspect(
        self,
        mut f: impl FnMut(&mut World, &T) + 'static,
    ) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).inspect(|val| f(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// None becomes Err(err). std: `Option::ok_or`
    pub fn ok_or<E: Clone + 'static>(
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
    pub fn ok_or_else<E>(
        self,
        mut f: impl FnMut(&mut World) -> E + 'static,
    ) -> PipelineBuilder<In, Result<T, E>, impl FnMut(&mut World, In) -> Result<T, E>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| chain(world, input).ok_or_else(|| f(world)),
            _marker: PhantomData,
        }
    }

    /// Exit Option — None becomes the default value.
    pub fn unwrap_or(self, default: T) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T>
    where
        T: Clone + 'static,
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
    pub fn unwrap_or_else(
        self,
        mut f: impl FnMut(&mut World) -> T + 'static,
    ) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).unwrap_or_else(|| f(world))
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
    // -- IntoStage-based (hot path) -------------------------------------------

    /// Transform the Ok value. Stage not called on Err.
    pub fn map<U, Params, S: IntoStage<T, U, Params>>(
        self,
        f: S,
        registry: &mut Registry,
    ) -> PipelineBuilder<
        In,
        Result<U, E>,
        impl FnMut(&mut World, In) -> Result<U, E> + use<In, T, E, U, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_stage(registry);
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).map(|val| resolved.call(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Short-circuits on Err. std: `Result::and_then`
    pub fn and_then<U, Params, S: IntoStage<T, Result<U, E>, Params>>(
        self,
        f: S,
        registry: &mut Registry,
    ) -> PipelineBuilder<
        In,
        Result<U, E>,
        impl FnMut(&mut World, In) -> Result<U, E> + use<In, T, E, U, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_stage(registry);
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
    pub fn catch<Params, S: IntoStage<E, (), Params>>(
        self,
        f: S,
        registry: &mut Registry,
    ) -> PipelineBuilder<
        In,
        Option<T>,
        impl FnMut(&mut World, In) -> Option<T> + use<In, T, E, Params, Chain, S>,
    > {
        let mut chain = self.chain;
        let mut resolved = f.into_stage(registry);
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

    // -- Closure-based (cold path, &mut World) --------------------------------

    /// Transform the error. std: `Result::map_err`
    pub fn map_err<E2>(
        self,
        mut f: impl FnMut(&mut World, E) -> E2 + 'static,
    ) -> PipelineBuilder<In, Result<T, E2>, impl FnMut(&mut World, In) -> Result<T, E2>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).map_err(|err| f(world, err))
            },
            _marker: PhantomData,
        }
    }

    /// Recover from Err. std: `Result::or_else`
    pub fn or_else<E2>(
        self,
        mut f: impl FnMut(&mut World, E) -> Result<T, E2> + 'static,
    ) -> PipelineBuilder<In, Result<T, E2>, impl FnMut(&mut World, In) -> Result<T, E2>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).or_else(|err| f(world, err))
            },
            _marker: PhantomData,
        }
    }

    /// Side effect on Ok. std: `Result::inspect`
    pub fn inspect(
        self,
        mut f: impl FnMut(&mut World, &T) + 'static,
    ) -> PipelineBuilder<In, Result<T, E>, impl FnMut(&mut World, In) -> Result<T, E>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).inspect(|val| f(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Side effect on Err. std: `Result::inspect_err`
    pub fn inspect_err(
        self,
        mut f: impl FnMut(&mut World, &E) + 'static,
    ) -> PipelineBuilder<In, Result<T, E>, impl FnMut(&mut World, In) -> Result<T, E>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).inspect_err(|err| f(world, err))
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
    pub fn unwrap_or(self, default: T) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T>
    where
        T: Clone + 'static,
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
    pub fn unwrap_or_else(
        self,
        mut f: impl FnMut(&mut World, E) -> T + 'static,
    ) -> PipelineBuilder<In, T, impl FnMut(&mut World, In) -> T> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| match chain(world, input) {
                Ok(val) => val,
                Err(err) => f(world, err),
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
/// If your pipeline produces a value, add a final `.stage()` that
/// writes it somewhere (e.g. `ResMut<T>`).
#[diagnostic::on_unimplemented(
    message = "`build()` requires the stage pipeline output to be `()`",
    label = "this pipeline produces `{Self}`, not `()`",
    note = "add a final `.stage()` that consumes the output"
)]
pub trait PipelineOutput {}
impl PipelineOutput for () {}

// =============================================================================
// build — when Out: PipelineOutput (i.e., Out = ())
// =============================================================================

impl<In: 'static, Out: PipelineOutput, Chain> PipelineBuilder<In, Out, Chain>
where
    Chain: FnMut(&mut World, In) -> Out + 'static,
{
    /// Box the composed closure and produce a [`Pipeline<In>`].
    ///
    /// Only available when the pipeline ends with `()`. If your chain
    /// produces a value, add a final `.stage()` that writes it to World.
    pub fn build(self) -> Pipeline<In> {
        let mut chain = self.chain;
        Pipeline {
            chain: Box::new(move |world, input| {
                chain(world, input);
            }),
        }
    }
}

// =============================================================================
// Pipeline<In> — built pipeline
// =============================================================================

/// Built stage pipeline implementing [`System<In>`](crate::System).
///
/// Created by [`PipelineBuilder::build`]. The entire pipeline chain is
/// monomorphized at compile time. `build()` erases the closure type via
/// `Box<dyn FnMut>` for the `dyn System<In>` boundary.
///
/// One virtual dispatch per `run()` call; the pipeline body executes
/// with zero further indirection — each stage does `get_unchecked` per
/// pre-resolved resource.
pub struct Pipeline<In> {
    chain: Box<dyn FnMut(&mut World, In)>,
}

impl<In: 'static> crate::System<In> for Pipeline<In> {
    fn run(&mut self, world: &mut World, event: In) {
        (self.chain)(world, event);
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Local, Res, ResMut, System, WorldBuilder};

    // =========================================================================
    // Core dispatch
    // =========================================================================

    #[test]
    fn stage_pure_transform() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().stage(|x: u32| x as u64 * 2, r);
        assert_eq!(p.run(&mut world, 5), 10u64);
    }

    #[test]
    fn stage_one_res() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        let mut world = wb.build();

        fn multiply(factor: Res<u64>, x: u32) -> u64 {
            *factor * x as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().stage(multiply, r);
        assert_eq!(p.run(&mut world, 5), 50);
    }

    #[test]
    fn stage_one_res_mut() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut total: ResMut<u64>, x: u32) {
            *total += x as u64;
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().stage(accumulate, r);
        p.run(&mut world, 10);
        p.run(&mut world, 5);
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn stage_two_params() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        wb.register::<bool>(true);
        let mut world = wb.build();

        fn conditional(factor: Res<u64>, flag: Res<bool>, x: u32) -> u64 {
            if *flag { *factor * x as u64 } else { 0 }
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().stage(conditional, r);
        assert_eq!(p.run(&mut world, 5), 50);
    }

    #[test]
    fn stage_chain_two() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(2);
        let mut world = wb.build();

        fn double(factor: Res<u64>, x: u32) -> u64 {
            *factor * x as u64
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(double, r)
            .stage(|val: u64| val + 1, r);
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
            .stage(|x: u32| -> Option<u32> { Some(x) }, r)
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
            .stage(|_x: u32| -> Option<u32> { None }, r)
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
            .stage(|x: u32| Some(x), r)
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
            .stage(|x: u32| Some(x), r)
            .and_then(check, r);
        assert_eq!(p.run(&mut world, 5), None);
    }

    #[test]
    fn option_on_none_fires() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Option<u32> { None }, r)
            .on_none(|w| {
                *w.resource_mut::<bool>() = true;
            });
        p.run(&mut world, 0);
        assert!(*world.resource::<bool>());
    }

    #[test]
    fn option_filter_keeps() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|x: u32| Some(x), r)
            .filter(|_w, x| *x > 3);
        assert_eq!(p.run(&mut world, 5), Some(5));
    }

    #[test]
    fn option_filter_drops() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|x: u32| Some(x), r)
            .filter(|_w, x| *x > 10);
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
            .stage(|x: u32| -> Result<u32, String> { Ok(x) }, r)
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
            .stage(|_x: u32| -> Result<u32, String> { Err("fail".into()) }, r)
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
            .stage(|_x: u32| -> Result<u32, String> { Err("caught".into()) }, r)
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
            .stage(|x: u32| -> Result<u32, String> { Ok(x) }, r)
            .catch(log_error, r);
        assert_eq!(p.run(&mut world, 5), Some(5));
        assert!(world.resource::<String>().is_empty());
    }

    // =========================================================================
    // Build + System
    // =========================================================================

    #[test]
    fn build_produces_system() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn accumulate(mut total: ResMut<u64>, x: u32) {
            *total += x as u64;
        }

        let r = world.registry_mut();
        let mut pipeline = PipelineStart::<u32>::new().stage(accumulate, r).build();

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
        let mut p = PipelineStart::<u32>::new().stage(multiply, r);
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
        let _p = PipelineStart::<u32>::new().stage(needs_u64, r);
    }

    // =========================================================================
    // Access conflict detection
    // =========================================================================

    #[test]
    #[should_panic(expected = "conflicting access")]
    fn stage_duplicate_access_panics() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn bad(a: Res<u64>, b: ResMut<u64>, _x: u32) -> u32 {
            let _ = (*a, &*b);
            0
        }

        let r = world.registry_mut();
        let _p = PipelineStart::<u32>::new().stage(bad, r);
    }

    // =========================================================================
    // Integration
    // =========================================================================

    #[test]
    fn local_in_stage() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        fn count(mut count: Local<u64>, mut total: ResMut<u64>, _x: u32) {
            *count += 1;
            *total = *count;
        }

        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new().stage(count, r);
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
            .stage(|x: u32| -> Option<u32> { Some(x) }, r)
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 5), 5);
    }

    #[test]
    fn option_unwrap_or_none() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Option<u32> { None }, r)
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 5), 99);
    }

    #[test]
    fn option_unwrap_or_else() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Option<u32> { None }, r)
            .unwrap_or_else(|_w| 42);
        assert_eq!(p.run(&mut world, 0), 42);
    }

    #[test]
    fn option_ok_or() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Option<u32> { None }, r)
            .ok_or("missing");
        assert_eq!(p.run(&mut world, 0), Err("missing"));
    }

    #[test]
    fn option_ok_or_some() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|x: u32| -> Option<u32> { Some(x) }, r)
            .ok_or("missing");
        assert_eq!(p.run(&mut world, 7), Ok(7));
    }

    #[test]
    fn option_ok_or_else() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Option<u32> { None }, r)
            .ok_or_else(|_w| "computed");
        assert_eq!(p.run(&mut world, 0), Err("computed"));
    }

    #[test]
    fn option_inspect_passes_through() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|x: u32| -> Option<u32> { Some(x) }, r)
            .inspect(|_w, _val| {});
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
            .stage(|_x: u32| -> Result<u32, i32> { Err(-1) }, r)
            .map_err(|_w, e| e.to_string());
        assert_eq!(p.run(&mut world, 0), Err("-1".to_string()));
    }

    #[test]
    fn result_map_err_ok_passthrough() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|x: u32| -> Result<u32, i32> { Ok(x) }, r)
            .map_err(|_w, e| e.to_string());
        assert_eq!(p.run(&mut world, 5), Ok(5));
    }

    #[test]
    fn result_or_else() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Result<u32, &str> { Err("fail") }, r)
            .or_else(|_w, _e| Ok::<u32, &str>(42));
        assert_eq!(p.run(&mut world, 0), Ok(42));
    }

    #[test]
    fn result_inspect_passes_through() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|x: u32| -> Result<u32, &str> { Ok(x) }, r)
            .inspect(|_w, _val| {});
        // inspect should pass through Ok unchanged.
        assert_eq!(p.run(&mut world, 7), Ok(7));
    }

    #[test]
    fn result_inspect_err_passes_through() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Result<u32, &str> { Err("bad") }, r)
            .inspect_err(|_w, _e| {});
        // inspect_err should pass through Err unchanged.
        assert_eq!(p.run(&mut world, 0), Err("bad"));
    }

    #[test]
    fn result_ok_converts() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|x: u32| -> Result<u32, &str> { Ok(x) }, r)
            .ok();
        assert_eq!(p.run(&mut world, 5), Some(5));
    }

    #[test]
    fn result_ok_drops_err() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Result<u32, &str> { Err("gone") }, r)
            .ok();
        assert_eq!(p.run(&mut world, 0), None);
    }

    #[test]
    fn result_unwrap_or() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Result<u32, &str> { Err("x") }, r)
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 0), 99);
    }

    #[test]
    fn result_unwrap_or_else() {
        let mut world = WorldBuilder::new().build();
        let r = world.registry_mut();
        let mut p = PipelineStart::<u32>::new()
            .stage(|_x: u32| -> Result<u32, i32> { Err(-5) }, r)
            .unwrap_or_else(|_w, e| e.unsigned_abs());
        assert_eq!(p.run(&mut world, 0), 5);
    }
}
