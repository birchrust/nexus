// Builder return types use complex generics for compile-time edge validation.
#![allow(clippy::type_complexity)]

//! DAG pipeline — monomorphized data-flow graphs with fan-out and merge.
//!
//! [`DagStart`] begins a typed DAG that encodes topology in the type system.
//! After monomorphization, the entire DAG is a single flat function with
//! all values as stack locals — no arena, no vtable dispatch, no unsafe.
//!
//! Nodes receive their input **by reference** — fan-out is free (multiple
//! arms borrow the same stack local). Nodes produce owned output values
//! passed to the next step.
//!
//! # When to use
//!
//! Use DAG pipelines when data needs to fan out to multiple arms and
//! merge back. For linear chains, prefer [`PipelineStart`](crate::PipelineStart).
//! For dynamic fan-out by reference, use [`FanOut`](crate::FanOut) or
//! [`Broadcast`](crate::Broadcast).
//!
//! # Flow control
//!
//! Option and Result combinators (`.guard()`, `.map()`, `.and_then()`,
//! `.filter()`, `.catch()`, etc.) work on both the main chain and
//! within arms.
//!
//! **Within an arm**, `None` / `Err` short-circuits the remaining steps
//! in **that arm only**. Sibling arms execute unconditionally. The merge
//! step receives whatever each arm produced (including `None`).
//!
//! To skip an entire fork, resolve Option/Result **before** `.fork()`:
//!
//! ```ignore
//! DagStart::<RawMsg>::new()
//!     .root(decode, reg)
//!     .guard(|_w, msg| msg.len() > 0)   // None skips everything below
//!     .unwrap_or(default)                // → T, enter fork with concrete type
//!     .fork()
//!     // arms work with &T, not &Option<T>
//! ```
//!
//! # Node signatures
//!
//! The root node takes the event by value. All other nodes take their
//! input by reference:
//!
//! ```ignore
//! // Root: event by value
//! fn decode(raw: RawMsg) -> DecodedMsg { .. }
//!
//! // Regular: input by reference
//! fn update_ob(msg: &DecodedMsg) { .. }
//! fn check_risk(config: Res<Config>, msg: &DecodedMsg) -> RiskResult { .. }
//! ```
//!
//! # Examples
//!
//! ```
//! use nexus_rt::{WorldBuilder, ResMut, Handler};
//! use nexus_rt::dag::DagStart;
//!
//! let mut wb = WorldBuilder::new();
//! wb.register::<u64>(0);
//! let mut world = wb.build();
//! let reg = world.registry();
//!
//! fn double(x: u32) -> u64 { x as u64 * 2 }
//! fn store(mut out: ResMut<u64>, val: &u64) { *out = *val; }
//!
//! let mut dag = DagStart::<u32>::new()
//!     .root(double, reg)
//!     .then(store, reg)
//!     .build();
//!
//! dag.run(&mut world, 5u32);
//! assert_eq!(*world.resource::<u64>(), 10);
//! ```

use std::marker::PhantomData;

use crate::Handler;
use crate::pipeline::{IntoStep, StepCall};
use crate::world::{Registry, World};

// =============================================================================
// MergeStepCall / IntoMergeStep — merge step dispatch
// =============================================================================

/// Callable trait for resolved merge steps.
///
/// Like [`StepCall`] but for merge steps with multiple reference inputs
/// bundled as `Inputs` (e.g. `(&'a A, &'a B)`).
#[doc(hidden)]
pub trait MergeStepCall<Inputs, Out> {
    /// Call this merge step with a world reference and input references.
    fn call(&mut self, world: &mut World, inputs: Inputs) -> Out;
}

/// Converts a named function into a resolved merge step.
///
/// Params first, then N reference inputs, returns output:
///
/// ```ignore
/// fn check(config: Res<Config>, ob: &ObResult, risk: &RiskResult) -> Decision { .. }
/// ```
#[doc(hidden)]
pub trait IntoMergeStep<Inputs, Out, Params> {
    /// The concrete resolved merge step type.
    type Step: MergeStepCall<Inputs, Out>;

    /// Resolve Param state from the registry and produce a merge step.
    fn into_merge_step(self, registry: &Registry) -> Self::Step;
}

/// Internal: pre-resolved merge step with cached Param state.
#[doc(hidden)]
pub struct MergeStep<F, Params: crate::handler::Param> {
    f: F,
    state: Params::State,
    #[allow(dead_code)]
    name: &'static str,
}

// -- Merge arity 2 -----------------------------------------------------------

// Param arity 0: closures work.
impl<A, B, Out, F> MergeStepCall<(&A, &B), Out> for MergeStep<F, ()>
where
    F: FnMut(&A, &B) -> Out + 'static,
{
    #[inline(always)]
    fn call(&mut self, _world: &mut World, inputs: (&A, &B)) -> Out {
        (self.f)(inputs.0, inputs.1)
    }
}

impl<A, B, Out, F> IntoMergeStep<(&A, &B), Out, ()> for F
where
    F: FnMut(&A, &B) -> Out + 'static,
{
    type Step = MergeStep<F, ()>;

    fn into_merge_step(self, registry: &Registry) -> Self::Step {
        MergeStep {
            f: self,
            state: <() as crate::handler::Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

// Param arities 1-8 for merge arity 2.
macro_rules! impl_merge2_step {
    ($($P:ident),+) => {
        impl<A, B, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            MergeStepCall<(&A, &B), Out> for MergeStep<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &A, &B) -> Out +
                FnMut($($P::Item<'a>,)+ &A, &B) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call(&mut self, world: &mut World, inputs: (&A, &B)) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ IA, IB, Output>(
                    mut f: impl FnMut($($P,)+ &IA, &IB) -> Output,
                    $($P: $P,)+
                    a: &IA, b: &IB,
                ) -> Output {
                    f($($P,)+ a, b)
                }
                let ($($P,)+) = unsafe {
                    <($($P,)+) as crate::handler::Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ inputs.0, inputs.1)
            }
        }

        impl<A, B, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            IntoMergeStep<(&A, &B), Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &A, &B) -> Out +
                FnMut($($P::Item<'a>,)+ &A, &B) -> Out,
        {
            type Step = MergeStep<F, ($($P,)+)>;

            fn into_merge_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as crate::handler::Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $((<$P as crate::handler::Param>::resource_id($P),
                           std::any::type_name::<$P>()),)+
                    ]);
                }
                MergeStep { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

// -- Merge arity 3 -----------------------------------------------------------

impl<A, B, C, Out, F> MergeStepCall<(&A, &B, &C), Out> for MergeStep<F, ()>
where
    F: FnMut(&A, &B, &C) -> Out + 'static,
{
    #[inline(always)]
    fn call(&mut self, _world: &mut World, inputs: (&A, &B, &C)) -> Out {
        (self.f)(inputs.0, inputs.1, inputs.2)
    }
}

impl<A, B, C, Out, F> IntoMergeStep<(&A, &B, &C), Out, ()> for F
where
    F: FnMut(&A, &B, &C) -> Out + 'static,
{
    type Step = MergeStep<F, ()>;

    fn into_merge_step(self, registry: &Registry) -> Self::Step {
        MergeStep {
            f: self,
            state: <() as crate::handler::Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

macro_rules! impl_merge3_step {
    ($($P:ident),+) => {
        impl<A, B, C, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            MergeStepCall<(&A, &B, &C), Out> for MergeStep<F, ($($P,)+)>
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &A, &B, &C) -> Out +
                FnMut($($P::Item<'a>,)+ &A, &B, &C) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call(&mut self, world: &mut World, inputs: (&A, &B, &C)) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ IA, IB, IC, Output>(
                    mut f: impl FnMut($($P,)+ &IA, &IB, &IC) -> Output,
                    $($P: $P,)+
                    a: &IA, b: &IB, c: &IC,
                ) -> Output {
                    f($($P,)+ a, b, c)
                }
                let ($($P,)+) = unsafe {
                    <($($P,)+) as crate::handler::Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ inputs.0, inputs.1, inputs.2)
            }
        }

        impl<A, B, C, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            IntoMergeStep<(&A, &B, &C), Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &A, &B, &C) -> Out +
                FnMut($($P::Item<'a>,)+ &A, &B, &C) -> Out,
        {
            type Step = MergeStep<F, ($($P,)+)>;

            fn into_merge_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as crate::handler::Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $((<$P as crate::handler::Param>::resource_id($P),
                           std::any::type_name::<$P>()),)+
                    ]);
                }
                MergeStep { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

// -- Merge arity 4 -----------------------------------------------------------

impl<A, B, C, D, Out, F> MergeStepCall<(&A, &B, &C, &D), Out> for MergeStep<F, ()>
where
    F: FnMut(&A, &B, &C, &D) -> Out + 'static,
{
    #[inline(always)]
    fn call(&mut self, _world: &mut World, i: (&A, &B, &C, &D)) -> Out {
        (self.f)(i.0, i.1, i.2, i.3)
    }
}

impl<A, B, C, D, Out, F> IntoMergeStep<(&A, &B, &C, &D), Out, ()> for F
where
    F: FnMut(&A, &B, &C, &D) -> Out + 'static,
{
    type Step = MergeStep<F, ()>;
    fn into_merge_step(self, registry: &Registry) -> Self::Step {
        MergeStep {
            f: self,
            state: <() as crate::handler::Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

macro_rules! impl_merge4_step {
    ($($P:ident),+) => {
        impl<A, B, C, D, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            MergeStepCall<(&A, &B, &C, &D), Out> for MergeStep<F, ($($P,)+)>
        where for<'a> &'a mut F:
            FnMut($($P,)+ &A, &B, &C, &D) -> Out +
            FnMut($($P::Item<'a>,)+ &A, &B, &C, &D) -> Out,
        {
            #[inline(always)]
            #[allow(non_snake_case)]
            fn call(&mut self, world: &mut World, i: (&A, &B, &C, &D)) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P,)+ IA, IB, IC, ID, Output>(
                    mut f: impl FnMut($($P,)+ &IA, &IB, &IC, &ID) -> Output,
                    $($P: $P,)+ a: &IA, b: &IB, c: &IC, d: &ID,
                ) -> Output { f($($P,)+ a, b, c, d) }
                let ($($P,)+) = unsafe {
                    <($($P,)+) as crate::handler::Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P,)+ i.0, i.1, i.2, i.3)
            }
        }
        impl<A, B, C, D, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            IntoMergeStep<(&A, &B, &C, &D), Out, ($($P,)+)> for F
        where for<'a> &'a mut F:
            FnMut($($P,)+ &A, &B, &C, &D) -> Out +
            FnMut($($P::Item<'a>,)+ &A, &B, &C, &D) -> Out,
        {
            type Step = MergeStep<F, ($($P,)+)>;
            fn into_merge_step(self, registry: &Registry) -> Self::Step {
                let state = <($($P,)+) as crate::handler::Param>::init(registry);
                { #[allow(non_snake_case)] let ($($P,)+) = &state;
                  registry.check_access(&[$((<$P as crate::handler::Param>::resource_id($P), std::any::type_name::<$P>()),)+]); }
                MergeStep { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

// -- all_tuples! for param arities -------------------------------------------

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

all_tuples!(impl_merge2_step);
all_tuples!(impl_merge3_step);
all_tuples!(impl_merge4_step);

// =============================================================================
// DAG — monomorphized, zero vtable dispatch
// =============================================================================
//
// Encodes DAG topology in the type system at compile time. After
// monomorphization, the entire DAG is a single flat function with all
// values as stack locals. No arena, no bitmap, no unsafe.
//
// Fan-out: multiple nodes borrow the same stack local (no Clone).
// Merge: merge step borrows all arm outputs.
// Panic safety: stack unwinding drops all locals automatically.

/// Entry point for building a DAG pipeline.
///
/// The DAG encodes topology in the type system at compile time,
/// producing a single monomorphized closure chain. All values live as
/// stack locals in the `run()` body — no arena, no vtable dispatch,
/// no unsafe.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, Handler};
/// use nexus_rt::dag::DagStart;
///
/// let mut wb = WorldBuilder::new();
/// wb.register::<u64>(0);
/// let mut world = wb.build();
/// let reg = world.registry();
///
/// fn double(x: u32) -> u64 { x as u64 * 2 }
/// fn store(mut out: ResMut<u64>, val: &u64) { *out = *val; }
///
/// let mut dag = DagStart::<u32>::new()
///     .root(double, reg)
///     .then(store, reg)
///     .build();
///
/// dag.run(&mut world, 5u32);
/// assert_eq!(*world.resource::<u64>(), 10);
/// ```
pub struct DagStart<E>(PhantomData<fn(E)>);

impl<E: 'static> DagStart<E> {
    /// Create a new typed DAG entry point.
    pub fn new() -> Self {
        Self(PhantomData)
    }

    /// Set the root step. Takes the event `E` by value, produces `Out`.
    pub fn root<Out, Params, S>(
        self,
        f: S,
        registry: &Registry,
    ) -> DagChain<E, Out, impl FnMut(&mut World, E) -> Out + use<E, Out, Params, S>>
    where
        Out: 'static,
        S: IntoStep<E, Out, Params>,
    {
        let mut resolved = f.into_step(registry);
        DagChain {
            chain: move |world: &mut World, event: E| resolved.call(world, event),
            _marker: PhantomData,
        }
    }
}

impl<E: 'static> Default for DagStart<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Main chain builder for a typed DAG.
///
/// `Chain` is `FnMut(&mut World, E) -> Out` — the monomorphized closure
/// representing all steps composed so far.
pub struct DagChain<E, Out, Chain> {
    chain: Chain,
    _marker: PhantomData<fn(E) -> Out>,
}

impl<E: 'static, Out: 'static, Chain> DagChain<E, Out, Chain>
where
    Chain: FnMut(&mut World, E) -> Out,
{
    /// Enter fork mode. Subsequent `.arm()` calls add parallel branches.
    pub fn fork(self) -> DagChainFork<E, Out, Chain, ()> {
        DagChainFork {
            chain: self.chain,
            arms: (),
            _marker: PhantomData,
        }
    }
}

impl<E: 'static, Chain> DagChain<E, (), Chain>
where
    Chain: FnMut(&mut World, E) + Send + 'static,
{
    /// Finalize into a [`Dag`](crate::Dag) that implements [`Handler<E>`].
    ///
    /// Only available when the chain ends with `()`. If your DAG
    /// produces a value, add a final `.then()` that consumes the output.
    pub fn build(self) -> Dag<E, Chain> {
        Dag {
            chain: self.chain,
            _marker: PhantomData,
        }
    }
}

/// Arm builder seed. Passed to `.arm()` closures.
///
/// Call `.then()` to add the first step in this arm.
pub struct DagArmStart<In>(PhantomData<fn(*const In)>);

impl<In: 'static> DagArmStart<In> {
    /// Add the first step in this arm. Takes `&In` by reference.
    pub fn then<Out, Params, S>(
        self,
        f: S,
        registry: &Registry,
    ) -> DagArm<In, Out, impl FnMut(&mut World, &In) -> Out + use<In, Out, Params, S>>
    where
        Out: 'static,
        S: IntoStep<&'static In, Out, Params>,
        S::Step: for<'a> StepCall<&'a In, Out>,
    {
        let mut resolved = f.into_step(registry);
        DagArm {
            chain: move |world: &mut World, input: &In| resolved.call(world, input),
            _marker: PhantomData,
        }
    }
}

/// Built arm in a typed DAG fork.
///
/// `Chain` is `FnMut(&mut World, &In) -> Out` — the monomorphized
/// closure for this arm's steps.
pub struct DagArm<In, Out, Chain> {
    chain: Chain,
    _marker: PhantomData<fn(*const In) -> Out>,
}

impl<In: 'static, Out: 'static, Chain> DagArm<In, Out, Chain>
where
    Chain: FnMut(&mut World, &In) -> Out,
{
    /// Enter fork mode within this arm.
    pub fn fork(self) -> DagArmFork<In, Out, Chain, ()> {
        DagArmFork {
            chain: self.chain,
            arms: (),
            _marker: PhantomData,
        }
    }
}

/// Fork builder on the main chain. Accumulates arms as a tuple.
pub struct DagChainFork<E, ForkOut, Chain, Arms> {
    chain: Chain,
    arms: Arms,
    _marker: PhantomData<fn(E) -> ForkOut>,
}

/// Fork builder within an arm. Accumulates sub-arms as a tuple.
pub struct DagArmFork<In, ForkOut, Chain, Arms> {
    chain: Chain,
    arms: Arms,
    _marker: PhantomData<fn(*const In) -> ForkOut>,
}

/// Final built DAG. Implements [`Handler<E>`].
///
/// Created by [`DagChain::build`]. The entire DAG is monomorphized
/// at compile time — no boxing, no virtual dispatch, no arena.
pub struct Dag<E, Chain> {
    chain: Chain,
    _marker: PhantomData<fn(E)>,
}

impl<E: 'static, Chain> Handler<E> for Dag<E, Chain>
where
    Chain: FnMut(&mut World, E) + Send + 'static,
{
    fn run(&mut self, world: &mut World, event: E) {
        (self.chain)(world, event);
    }

    fn name(&self) -> &'static str {
        "dag::Dag"
    }
}

// =============================================================================
// Fork arity macro — arm accumulation, merge, join
// =============================================================================

// =============================================================================
// Combinator macro — shared between DagChain and DagArm
// =============================================================================

/// Generates step combinators, Option/Result helpers, and clone helpers.
///
/// DagChain and DagArm differ only in how the upstream chain is
/// called (by value vs by reference). This macro generates identical
/// combinator sets for both.
///
/// All `IntoStep`-based methods resolve steps with `&T` input (DAG
/// semantics — every step borrows its input, never consumes it).
macro_rules! impl_dag_combinators {
    (
        builder: $Builder:ident,
        upstream: $U:ident,
        chain_input: $chain_input:ty,
        param: $pname:ident : $pty:ty
    ) => {
        // =============================================================
        // Core — any Out
        // =============================================================

        impl<$U: 'static, Out: 'static, Chain> $Builder<$U, Out, Chain>
        where
            Chain: FnMut(&mut World, $chain_input) -> Out,
        {
            /// Append a step. The step receives `&Out` by reference.
            pub fn then<NewOut, Params, S>(
                self,
                f: S,
                registry: &Registry,
            ) -> $Builder<
                $U,
                NewOut,
                impl FnMut(&mut World, $pty) -> NewOut
                    + use<$U, Out, NewOut, Params, Chain, S>,
            >
            where
                NewOut: 'static,
                S: IntoStep<&'static Out, NewOut, Params>,
                S::Step: for<'a> StepCall<&'a Out, NewOut>,
            {
                let mut chain = self.chain;
                let mut resolved = f.into_step(registry);
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        let out = chain(world, $pname);
                        resolved.call(world, &out)
                    },
                    _marker: PhantomData,
                }
            }

            /// Dispatch output to a [`Handler<Out>`].
            ///
            /// Feeds the chain's output into any handler —
            /// [`HandlerFn`](crate::HandlerFn), [`Callback`](crate::Callback),
            /// [`Pipeline`](crate::Pipeline), etc.
            pub fn dispatch<H: Handler<Out>>(
                self,
                mut handler: H,
            ) -> $Builder<$U, (), impl FnMut(&mut World, $pty) + use<$U, Out, Chain, H>>
            {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        let out = chain(world, $pname);
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
            ///
            /// Within a DAG arm, `None` short-circuits the remaining arm
            /// steps — sibling arms and the merge step still execute.
            pub fn guard(
                self,
                mut f: impl FnMut(&mut World, &Out) -> bool + 'static,
            ) -> $Builder<
                $U,
                Option<Out>,
                impl FnMut(&mut World, $pty) -> Option<Out>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        let val = chain(world, $pname);
                        if f(world, &val) { Some(val) } else { None }
                    },
                    _marker: PhantomData,
                }
            }
        }

        // =============================================================
        // Clone helpers — &T → T transitions
        // =============================================================

        impl<'a, $U: 'static, T: Clone, Chain> $Builder<$U, &'a T, Chain>
        where
            Chain: FnMut(&mut World, $chain_input) -> &'a T,
        {
            /// Clone a borrowed output to produce an owned value.
            ///
            /// Uses UFCS (`T::clone(val)`) — `val.clone()` on `&&T`
            /// resolves to `<&T as Clone>::clone`, returning `&T` not `T`.
            pub fn cloned(
                self,
            ) -> $Builder<$U, T, impl FnMut(&mut World, $pty) -> T> {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        T::clone(chain(world, $pname))
                    },
                    _marker: PhantomData,
                }
            }
        }

        impl<'a, $U: 'static, T: Clone, Chain> $Builder<$U, Option<&'a T>, Chain>
        where
            Chain: FnMut(&mut World, $chain_input) -> Option<&'a T>,
        {
            /// Clone inner borrowed value. `Option<&T>` → `Option<T>`.
            pub fn cloned(
                self,
            ) -> $Builder<$U, Option<T>, impl FnMut(&mut World, $pty) -> Option<T>>
            {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname).cloned()
                    },
                    _marker: PhantomData,
                }
            }
        }

        impl<'a, $U: 'static, T: Clone, Err, Chain>
            $Builder<$U, Result<&'a T, Err>, Chain>
        where
            Chain: FnMut(&mut World, $chain_input) -> Result<&'a T, Err>,
        {
            /// Clone inner borrowed Ok value.
            /// `Result<&T, Err>` → `Result<T, Err>`.
            pub fn cloned(
                self,
            ) -> $Builder<
                $U,
                Result<T, Err>,
                impl FnMut(&mut World, $pty) -> Result<T, Err>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname).cloned()
                    },
                    _marker: PhantomData,
                }
            }
        }

        // =============================================================
        // Option helpers — $Builder<$U, Option<T>, Chain>
        // =============================================================

        impl<$U: 'static, T: 'static, Chain> $Builder<$U, Option<T>, Chain>
        where
            Chain: FnMut(&mut World, $chain_input) -> Option<T>,
        {
            // -- IntoStep-based (hot path) --------------------------------

            /// Transform the inner value. Step not called on None.
            pub fn map<U, Params, S: IntoStep<&'static T, U, Params>>(
                self,
                f: S,
                registry: &Registry,
            ) -> $Builder<
                $U,
                Option<U>,
                impl FnMut(&mut World, $pty) -> Option<U>
                    + use<$U, T, U, Params, Chain, S>,
            >
            where
                U: 'static,
                S::Step: for<'x> StepCall<&'x T, U>,
            {
                let mut chain = self.chain;
                let mut resolved = f.into_step(registry);
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .map(|ref val| resolved.call(world, val))
                    },
                    _marker: PhantomData,
                }
            }

            /// Short-circuits on None. std: `Option::and_then`
            pub fn and_then<
                U,
                Params,
                S: IntoStep<&'static T, Option<U>, Params>,
            >(
                self,
                f: S,
                registry: &Registry,
            ) -> $Builder<
                $U,
                Option<U>,
                impl FnMut(&mut World, $pty) -> Option<U>
                    + use<$U, T, U, Params, Chain, S>,
            >
            where
                U: 'static,
                S::Step: for<'x> StepCall<&'x T, Option<U>>,
            {
                let mut chain = self.chain;
                let mut resolved = f.into_step(registry);
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .and_then(|ref val| resolved.call(world, val))
                    },
                    _marker: PhantomData,
                }
            }

            // -- Closure-based (cold path) --------------------------------

            /// Side effect on None.
            pub fn on_none(
                self,
                mut f: impl FnMut(&mut World) + 'static,
            ) -> $Builder<
                $U,
                Option<T>,
                impl FnMut(&mut World, $pty) -> Option<T>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        let result = chain(world, $pname);
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
            ) -> $Builder<
                $U,
                Option<T>,
                impl FnMut(&mut World, $pty) -> Option<T>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname).filter(|val| f(world, val))
                    },
                    _marker: PhantomData,
                }
            }

            /// Side effect on Some value. std: `Option::inspect`
            pub fn inspect(
                self,
                mut f: impl FnMut(&mut World, &T) + 'static,
            ) -> $Builder<
                $U,
                Option<T>,
                impl FnMut(&mut World, $pty) -> Option<T>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname).inspect(|val| f(world, val))
                    },
                    _marker: PhantomData,
                }
            }

            /// None becomes Err(err). std: `Option::ok_or`
            pub fn ok_or<Err: Clone + 'static>(
                self,
                err: Err,
            ) -> $Builder<
                $U,
                Result<T, Err>,
                impl FnMut(&mut World, $pty) -> Result<T, Err>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname).ok_or_else(|| err.clone())
                    },
                    _marker: PhantomData,
                }
            }

            /// None becomes Err(f()). std: `Option::ok_or_else`
            pub fn ok_or_else<Err>(
                self,
                mut f: impl FnMut(&mut World) -> Err + 'static,
            ) -> $Builder<
                $U,
                Result<T, Err>,
                impl FnMut(&mut World, $pty) -> Result<T, Err>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname).ok_or_else(|| f(world))
                    },
                    _marker: PhantomData,
                }
            }

            /// Exit Option — None becomes the default value.
            pub fn unwrap_or(
                self,
                default: T,
            ) -> $Builder<$U, T, impl FnMut(&mut World, $pty) -> T>
            where
                T: Clone,
            {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .unwrap_or_else(|| default.clone())
                    },
                    _marker: PhantomData,
                }
            }

            /// Exit Option — None becomes `f()`.
            pub fn unwrap_or_else(
                self,
                mut f: impl FnMut(&mut World) -> T + 'static,
            ) -> $Builder<$U, T, impl FnMut(&mut World, $pty) -> T>
            {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname).unwrap_or_else(|| f(world))
                    },
                    _marker: PhantomData,
                }
            }
        }

        // =============================================================
        // Result helpers — $Builder<$U, Result<T, Err>, Chain>
        // =============================================================

        impl<$U: 'static, T: 'static, Err: 'static, Chain>
            $Builder<$U, Result<T, Err>, Chain>
        where
            Chain: FnMut(&mut World, $chain_input) -> Result<T, Err>,
        {
            // -- IntoStep-based (hot path) --------------------------------

            /// Transform the Ok value. Step not called on Err.
            pub fn map<U, Params, S: IntoStep<&'static T, U, Params>>(
                self,
                f: S,
                registry: &Registry,
            ) -> $Builder<
                $U,
                Result<U, Err>,
                impl FnMut(&mut World, $pty) -> Result<U, Err>
                    + use<$U, T, Err, U, Params, Chain, S>,
            >
            where
                U: 'static,
                S::Step: for<'x> StepCall<&'x T, U>,
            {
                let mut chain = self.chain;
                let mut resolved = f.into_step(registry);
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .map(|ref val| resolved.call(world, val))
                    },
                    _marker: PhantomData,
                }
            }

            /// Short-circuits on Err. std: `Result::and_then`
            pub fn and_then<
                U,
                Params,
                S: IntoStep<&'static T, Result<U, Err>, Params>,
            >(
                self,
                f: S,
                registry: &Registry,
            ) -> $Builder<
                $U,
                Result<U, Err>,
                impl FnMut(&mut World, $pty) -> Result<U, Err>
                    + use<$U, T, Err, U, Params, Chain, S>,
            >
            where
                U: 'static,
                S::Step: for<'x> StepCall<&'x T, Result<U, Err>>,
            {
                let mut chain = self.chain;
                let mut resolved = f.into_step(registry);
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .and_then(|ref val| resolved.call(world, val))
                    },
                    _marker: PhantomData,
                }
            }

            /// Handle error and transition to Option.
            ///
            /// `Ok(val)` becomes `Some(val)` — handler not called.
            /// `Err(err)` calls the handler, then produces `None`.
            pub fn catch<Params, S: IntoStep<&'static Err, (), Params>>(
                self,
                f: S,
                registry: &Registry,
            ) -> $Builder<
                $U,
                Option<T>,
                impl FnMut(&mut World, $pty) -> Option<T>
                    + use<$U, T, Err, Params, Chain, S>,
            >
            where
                S::Step: for<'x> StepCall<&'x Err, ()>,
            {
                let mut chain = self.chain;
                let mut resolved = f.into_step(registry);
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        match chain(world, $pname) {
                            Ok(val) => Some(val),
                            Err(ref err) => {
                                resolved.call(world, err);
                                None
                            }
                        }
                    },
                    _marker: PhantomData,
                }
            }

            // -- Closure-based (cold path) --------------------------------

            /// Transform the error. std: `Result::map_err`
            pub fn map_err<Err2>(
                self,
                mut f: impl FnMut(&mut World, Err) -> Err2 + 'static,
            ) -> $Builder<
                $U,
                Result<T, Err2>,
                impl FnMut(&mut World, $pty) -> Result<T, Err2>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .map_err(|err| f(world, err))
                    },
                    _marker: PhantomData,
                }
            }

            /// Recover from Err. std: `Result::or_else`
            pub fn or_else<Err2>(
                self,
                mut f: impl FnMut(&mut World, Err) -> Result<T, Err2>
                    + 'static,
            ) -> $Builder<
                $U,
                Result<T, Err2>,
                impl FnMut(&mut World, $pty) -> Result<T, Err2>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .or_else(|err| f(world, err))
                    },
                    _marker: PhantomData,
                }
            }

            /// Side effect on Ok. std: `Result::inspect`
            pub fn inspect(
                self,
                mut f: impl FnMut(&mut World, &T) + 'static,
            ) -> $Builder<
                $U,
                Result<T, Err>,
                impl FnMut(&mut World, $pty) -> Result<T, Err>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .inspect(|val| f(world, val))
                    },
                    _marker: PhantomData,
                }
            }

            /// Side effect on Err. std: `Result::inspect_err`
            pub fn inspect_err(
                self,
                mut f: impl FnMut(&mut World, &Err) + 'static,
            ) -> $Builder<
                $U,
                Result<T, Err>,
                impl FnMut(&mut World, $pty) -> Result<T, Err>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .inspect_err(|err| f(world, err))
                    },
                    _marker: PhantomData,
                }
            }

            /// Discard error, enter Option land. std: `Result::ok`
            pub fn ok(
                self,
            ) -> $Builder<
                $U,
                Option<T>,
                impl FnMut(&mut World, $pty) -> Option<T>,
            > {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname).ok()
                    },
                    _marker: PhantomData,
                }
            }

            /// Exit Result — Err becomes the default value.
            pub fn unwrap_or(
                self,
                default: T,
            ) -> $Builder<$U, T, impl FnMut(&mut World, $pty) -> T>
            where
                T: Clone,
            {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        chain(world, $pname)
                            .unwrap_or_else(|_| default.clone())
                    },
                    _marker: PhantomData,
                }
            }

            /// Exit Result — Err becomes `f(err)`.
            pub fn unwrap_or_else(
                self,
                mut f: impl FnMut(&mut World, Err) -> T + 'static,
            ) -> $Builder<$U, T, impl FnMut(&mut World, $pty) -> T>
            {
                let mut chain = self.chain;
                $Builder {
                    chain: move |world: &mut World, $pname: $pty| {
                        match chain(world, $pname) {
                            Ok(val) => val,
                            Err(err) => f(world, err),
                        }
                    },
                    _marker: PhantomData,
                }
            }
        }
    };
}

impl_dag_combinators!(
    builder: DagChain,
    upstream: E,
    chain_input: E,
    param: event: E
);

impl_dag_combinators!(
    builder: DagArm,
    upstream: In,
    chain_input: &In,
    param: input: &In
);

// =============================================================================
// Fork arity macro — arm accumulation, merge, join
// =============================================================================

/// Generates arm accumulation, merge, and join for a fork type.
///
/// ChainFork and ArmFork differ only in:
/// - How the upstream chain is called (by value vs by reference)
/// - What output type is produced (DagChain vs DagArm)
macro_rules! impl_dag_fork {
    (
        fork: $Fork:ident,
        output: $Output:ident,
        upstream: $U:ident,
        chain_input: $chain_input:ty,
        param: $pname:ident : $pty:ty
    ) => {
        // =============================================================
        // Arm accumulation: 0→1, 1→2, 2→3, 3→4
        // =============================================================

        impl<$U, ForkOut, Chain> $Fork<$U, ForkOut, Chain, ()> {
            /// Add the first arm to this fork.
            pub fn arm<AOut, ACh>(
                self,
                f: impl FnOnce(DagArmStart<ForkOut>) -> DagArm<ForkOut, AOut, ACh>,
            ) -> $Fork<$U, ForkOut, Chain, (DagArm<ForkOut, AOut, ACh>,)> {
                let arm = f(DagArmStart(PhantomData));
                $Fork {
                    chain: self.chain,
                    arms: (arm,),
                    _marker: PhantomData,
                }
            }
        }

        impl<$U, ForkOut, Chain, A0, C0> $Fork<$U, ForkOut, Chain, (DagArm<ForkOut, A0, C0>,)> {
            /// Add a second arm to this fork.
            pub fn arm<AOut, ACh>(
                self,
                f: impl FnOnce(DagArmStart<ForkOut>) -> DagArm<ForkOut, AOut, ACh>,
            ) -> $Fork<$U, ForkOut, Chain, (DagArm<ForkOut, A0, C0>, DagArm<ForkOut, AOut, ACh>)>
            {
                let arm = f(DagArmStart(PhantomData));
                let (a0,) = self.arms;
                $Fork {
                    chain: self.chain,
                    arms: (a0, arm),
                    _marker: PhantomData,
                }
            }
        }

        impl<$U, ForkOut, Chain, A0, C0, A1, C1>
            $Fork<$U, ForkOut, Chain, (DagArm<ForkOut, A0, C0>, DagArm<ForkOut, A1, C1>)>
        {
            /// Add a third arm to this fork.
            pub fn arm<AOut, ACh>(
                self,
                f: impl FnOnce(DagArmStart<ForkOut>) -> DagArm<ForkOut, AOut, ACh>,
            ) -> $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    DagArm<ForkOut, A0, C0>,
                    DagArm<ForkOut, A1, C1>,
                    DagArm<ForkOut, AOut, ACh>,
                ),
            > {
                let arm = f(DagArmStart(PhantomData));
                let (a0, a1) = self.arms;
                $Fork {
                    chain: self.chain,
                    arms: (a0, a1, arm),
                    _marker: PhantomData,
                }
            }
        }

        impl<$U, ForkOut, Chain, A0, C0, A1, C1, A2, C2>
            $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    DagArm<ForkOut, A0, C0>,
                    DagArm<ForkOut, A1, C1>,
                    DagArm<ForkOut, A2, C2>,
                ),
            >
        {
            /// Add a fourth arm to this fork.
            pub fn arm<AOut, ACh>(
                self,
                f: impl FnOnce(DagArmStart<ForkOut>) -> DagArm<ForkOut, AOut, ACh>,
            ) -> $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    DagArm<ForkOut, A0, C0>,
                    DagArm<ForkOut, A1, C1>,
                    DagArm<ForkOut, A2, C2>,
                    DagArm<ForkOut, AOut, ACh>,
                ),
            > {
                let arm = f(DagArmStart(PhantomData));
                let (a0, a1, a2) = self.arms;
                $Fork {
                    chain: self.chain,
                    arms: (a0, a1, a2, arm),
                    _marker: PhantomData,
                }
            }
        }

        // =============================================================
        // Merge arity 2
        // =============================================================

        impl<$U: 'static, ForkOut: 'static, Chain, A0: 'static, C0, A1: 'static, C1>
            $Fork<$U, ForkOut, Chain, (DagArm<ForkOut, A0, C0>, DagArm<ForkOut, A1, C1>)>
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut) -> A0,
            C1: FnMut(&mut World, &ForkOut) -> A1,
        {
            /// Merge two arms with a merge step.
            pub fn merge<MOut, Params, S>(
                self,
                f: S,
                registry: &Registry,
            ) -> $Output<
                $U,
                MOut,
                impl FnMut(&mut World, $pty) -> MOut
                + use<$U, ForkOut, MOut, Params, Chain, S, A0, C0, A1, C1>,
            >
            where
                MOut: 'static,
                S: IntoMergeStep<(&'static A0, &'static A1), MOut, Params>,
                S::Step: for<'x> MergeStepCall<(&'x A0, &'x A1), MOut>,
            {
                let mut chain = self.chain;
                let (a0, a1) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                let mut ms = f.into_merge_step(registry);
                $Output {
                    chain: move |world: &mut World, $pname: $pty| {
                        let fork_out = chain(world, $pname);
                        let o0 = c0(world, &fork_out);
                        let o1 = c1(world, &fork_out);
                        ms.call(world, (&o0, &o1))
                    },
                    _marker: PhantomData,
                }
            }
        }

        impl<$U: 'static, ForkOut: 'static, Chain, C0, C1>
            $Fork<$U, ForkOut, Chain, (DagArm<ForkOut, (), C0>, DagArm<ForkOut, (), C1>)>
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut),
            C1: FnMut(&mut World, &ForkOut),
        {
            /// Join two sink arms (all producing `()`).
            pub fn join(
                self,
            ) -> $Output<$U, (), impl FnMut(&mut World, $pty) + use<$U, ForkOut, Chain, C0, C1>>
            {
                let mut chain = self.chain;
                let (a0, a1) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                $Output {
                    chain: move |world: &mut World, $pname: $pty| {
                        let fork_out = chain(world, $pname);
                        c0(world, &fork_out);
                        c1(world, &fork_out);
                    },
                    _marker: PhantomData,
                }
            }
        }

        // =============================================================
        // Merge arity 3
        // =============================================================

        impl<
            $U: 'static,
            ForkOut: 'static,
            Chain,
            A0: 'static,
            C0,
            A1: 'static,
            C1,
            A2: 'static,
            C2,
        >
            $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    DagArm<ForkOut, A0, C0>,
                    DagArm<ForkOut, A1, C1>,
                    DagArm<ForkOut, A2, C2>,
                ),
            >
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut) -> A0,
            C1: FnMut(&mut World, &ForkOut) -> A1,
            C2: FnMut(&mut World, &ForkOut) -> A2,
        {
            /// Merge three arms with a merge step.
            pub fn merge<MOut, Params, S>(
                self,
                f: S,
                registry: &Registry,
            ) -> $Output<
                $U,
                MOut,
                impl FnMut(&mut World, $pty) -> MOut
                + use<$U, ForkOut, MOut, Params, Chain, S, A0, C0, A1, C1, A2, C2>,
            >
            where
                MOut: 'static,
                S: IntoMergeStep<(&'static A0, &'static A1, &'static A2), MOut, Params>,
                S::Step: for<'x> MergeStepCall<(&'x A0, &'x A1, &'x A2), MOut>,
            {
                let mut chain = self.chain;
                let (a0, a1, a2) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                let mut c2 = a2.chain;
                let mut ms = f.into_merge_step(registry);
                $Output {
                    chain: move |world: &mut World, $pname: $pty| {
                        let fork_out = chain(world, $pname);
                        let o0 = c0(world, &fork_out);
                        let o1 = c1(world, &fork_out);
                        let o2 = c2(world, &fork_out);
                        ms.call(world, (&o0, &o1, &o2))
                    },
                    _marker: PhantomData,
                }
            }
        }

        impl<$U: 'static, ForkOut: 'static, Chain, C0, C1, C2>
            $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    DagArm<ForkOut, (), C0>,
                    DagArm<ForkOut, (), C1>,
                    DagArm<ForkOut, (), C2>,
                ),
            >
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut),
            C1: FnMut(&mut World, &ForkOut),
            C2: FnMut(&mut World, &ForkOut),
        {
            /// Join three sink arms (all producing `()`).
            pub fn join(
                self,
            ) -> $Output<$U, (), impl FnMut(&mut World, $pty) + use<$U, ForkOut, Chain, C0, C1, C2>>
            {
                let mut chain = self.chain;
                let (a0, a1, a2) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                let mut c2 = a2.chain;
                $Output {
                    chain: move |world: &mut World, $pname: $pty| {
                        let fork_out = chain(world, $pname);
                        c0(world, &fork_out);
                        c1(world, &fork_out);
                        c2(world, &fork_out);
                    },
                    _marker: PhantomData,
                }
            }
        }

        // =============================================================
        // Merge arity 4
        // =============================================================

        #[allow(clippy::many_single_char_names)]
        impl<
            $U: 'static,
            ForkOut: 'static,
            Chain,
            A0: 'static,
            C0,
            A1: 'static,
            C1,
            A2: 'static,
            C2,
            A3: 'static,
            C3,
        >
            $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    DagArm<ForkOut, A0, C0>,
                    DagArm<ForkOut, A1, C1>,
                    DagArm<ForkOut, A2, C2>,
                    DagArm<ForkOut, A3, C3>,
                ),
            >
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut) -> A0,
            C1: FnMut(&mut World, &ForkOut) -> A1,
            C2: FnMut(&mut World, &ForkOut) -> A2,
            C3: FnMut(&mut World, &ForkOut) -> A3,
        {
            /// Merge four arms with a merge step.
            pub fn merge<MOut, Params, S>(
                self,
                f: S,
                registry: &Registry,
            ) -> $Output<
                $U,
                MOut,
                impl FnMut(&mut World, $pty) -> MOut
                + use<$U, ForkOut, MOut, Params, Chain, S, A0, C0, A1, C1, A2, C2, A3, C3>,
            >
            where
                MOut: 'static,
                S: IntoMergeStep<
                        (&'static A0, &'static A1, &'static A2, &'static A3),
                        MOut,
                        Params,
                    >,
                S::Step: for<'x> MergeStepCall<(&'x A0, &'x A1, &'x A2, &'x A3), MOut>,
            {
                let mut chain = self.chain;
                let (a0, a1, a2, a3) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                let mut c2 = a2.chain;
                let mut c3 = a3.chain;
                let mut ms = f.into_merge_step(registry);
                $Output {
                    chain: move |world: &mut World, $pname: $pty| {
                        let fork_out = chain(world, $pname);
                        let o0 = c0(world, &fork_out);
                        let o1 = c1(world, &fork_out);
                        let o2 = c2(world, &fork_out);
                        let o3 = c3(world, &fork_out);
                        ms.call(world, (&o0, &o1, &o2, &o3))
                    },
                    _marker: PhantomData,
                }
            }
        }

        impl<$U: 'static, ForkOut: 'static, Chain, C0, C1, C2, C3>
            $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    DagArm<ForkOut, (), C0>,
                    DagArm<ForkOut, (), C1>,
                    DagArm<ForkOut, (), C2>,
                    DagArm<ForkOut, (), C3>,
                ),
            >
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut),
            C1: FnMut(&mut World, &ForkOut),
            C2: FnMut(&mut World, &ForkOut),
            C3: FnMut(&mut World, &ForkOut),
        {
            /// Join four sink arms (all producing `()`).
            pub fn join(
                self,
            ) -> $Output<
                $U,
                (),
                impl FnMut(&mut World, $pty) + use<$U, ForkOut, Chain, C0, C1, C2, C3>,
            > {
                let mut chain = self.chain;
                let (a0, a1, a2, a3) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                let mut c2 = a2.chain;
                let mut c3 = a3.chain;
                $Output {
                    chain: move |world: &mut World, $pname: $pty| {
                        let fork_out = chain(world, $pname);
                        c0(world, &fork_out);
                        c1(world, &fork_out);
                        c2(world, &fork_out);
                        c3(world, &fork_out);
                    },
                    _marker: PhantomData,
                }
            }
        }
    };
}

impl_dag_fork!(
    fork: DagChainFork,
    output: DagChain,
    upstream: E,
    chain_input: E,
    param: event: E
);

impl_dag_fork!(
    fork: DagArmFork,
    output: DagArm,
    upstream: In,
    chain_input: &In,
    param: input: &In
);

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoHandler, Res, ResMut, Virtual, WorldBuilder};

    // -- Linear chains --

    #[test]
    fn dag_linear_2() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_mul2(x: u32) -> u64 {
            x as u64 * 2
        }
        fn store(mut out: ResMut<u64>, val: &u64) {
            *out = *val;
        }

        let mut dag = DagStart::<u32>::new()
            .root(root_mul2, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 10);
    }

    #[test]
    fn dag_linear_3() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_mul2(x: u32) -> u64 {
            x as u64 * 2
        }
        fn add_one(val: &u64) -> u64 {
            *val + 1
        }
        fn store(mut out: ResMut<u64>, val: &u64) {
            *out = *val;
        }

        let mut dag = DagStart::<u32>::new()
            .root(root_mul2, reg)
            .then(add_one, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 11); // (5*2)+1
    }

    #[test]
    fn dag_linear_5() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_id(x: u32) -> u64 {
            x as u64
        }
        fn add_one(val: &u64) -> u64 {
            *val + 1
        }
        fn store(mut out: ResMut<u64>, val: &u64) {
            *out = *val;
        }

        let mut dag = DagStart::<u32>::new()
            .root(root_id, reg)
            .then(add_one, reg)
            .then(add_one, reg)
            .then(add_one, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 3); // 0+1+1+1
    }

    // -- Diamond: root → [a, b] → merge → sink --

    #[test]
    fn dag_diamond() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_mul2(x: u32) -> u32 {
            x.wrapping_mul(2)
        }
        fn add_one(val: &u32) -> u32 {
            val.wrapping_add(1)
        }
        fn mul3(val: &u32) -> u32 {
            val.wrapping_mul(3)
        }
        fn merge_add(a: &u32, b: &u32) -> u32 {
            a.wrapping_add(*b)
        }
        fn store(mut out: ResMut<u64>, val: &u32) {
            *out = *val as u64;
        }

        let mut dag = DagStart::<u32>::new()
            .root(root_mul2, reg)
            .fork()
            .arm(|a| a.then(add_one, reg))
            .arm(|b| b.then(mul3, reg))
            .merge(merge_add, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 5u32);
        // root: 10, arm_a: 11, arm_b: 30, merge: 41
        assert_eq!(*world.resource::<u64>(), 41);
    }

    // -- Fan-out to sinks (.join()) --

    #[test]
    fn dag_fan_out_join() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<i64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_id(x: u32) -> u64 {
            x as u64
        }
        fn sink_u64(mut out: ResMut<u64>, val: &u64) {
            *out = *val * 2;
        }
        fn sink_i64(mut out: ResMut<i64>, val: &u64) {
            *out = *val as i64 * 3;
        }

        let mut dag = DagStart::<u32>::new()
            .root(root_id, reg)
            .fork()
            .arm(|a| a.then(sink_u64, reg))
            .arm(|b| b.then(sink_i64, reg))
            .join()
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 10);
        assert_eq!(*world.resource::<i64>(), 15);
    }

    // -- Nested fork within an arm --

    #[test]
    fn dag_nested_fork() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_id(x: u32) -> u32 {
            x
        }
        fn add_10(val: &u32) -> u32 {
            val.wrapping_add(10)
        }
        fn mul2(val: &u32) -> u32 {
            val.wrapping_mul(2)
        }
        fn mul3(val: &u32) -> u32 {
            val.wrapping_mul(3)
        }
        fn inner_merge(a: &u32, b: &u32) -> u32 {
            a.wrapping_add(*b)
        }
        fn outer_merge(a: &u32, b: &u32) -> u32 {
            a.wrapping_add(*b)
        }
        fn store(mut out: ResMut<u64>, val: &u32) {
            *out = *val as u64;
        }

        // root(5)=5 → fork
        //   arm_a: add_10(5)=15 → fork
        //     sub_c: mul2(15)=30
        //     sub_d: mul3(15)=45
        //     inner_merge(30,45)=75
        //   arm_b: mul3(5)=15
        // outer_merge(75,15)=90
        let mut dag = DagStart::<u32>::new()
            .root(root_id, reg)
            .fork()
            .arm(|a| {
                a.then(add_10, reg)
                    .fork()
                    .arm(|c| c.then(mul2, reg))
                    .arm(|d| d.then(mul3, reg))
                    .merge(inner_merge, reg)
            })
            .arm(|b| b.then(mul3, reg))
            .merge(outer_merge, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 90);
    }

    // -- Complex topology: asymmetric arm lengths --

    #[test]
    fn dag_complex_topology() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_mul2(x: u32) -> u32 {
            x.wrapping_mul(2)
        }
        fn add_one(val: &u32) -> u32 {
            val.wrapping_add(1)
        }
        fn add_then_mul2(val: &u32) -> u32 {
            val.wrapping_add(1).wrapping_mul(2)
        }
        fn mul3(val: &u32) -> u32 {
            val.wrapping_mul(3)
        }
        fn merge_add(a: &u32, b: &u32) -> u32 {
            a.wrapping_add(*b)
        }
        fn store(mut out: ResMut<u64>, val: &u32) {
            *out = *val as u64;
        }

        // root(5)=10 → fork
        //   a: add_one(10)=11 → add_then_mul2(11)=24
        //   b: mul3(10)=30
        // merge(24, 30) = 54
        let mut dag = DagStart::<u32>::new()
            .root(root_mul2, reg)
            .fork()
            .arm(|a| a.then(add_one, reg).then(add_then_mul2, reg))
            .arm(|b| b.then(mul3, reg))
            .merge(merge_add, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 54);
    }

    // -- Boxable into Box<dyn Handler<E>> --

    #[test]
    fn dag_boxable() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_id(x: u32) -> u64 {
            x as u64
        }
        fn store(mut out: ResMut<u64>, val: &u64) {
            *out = *val;
        }

        let mut boxed: Virtual<u32> = Box::new(
            DagStart::<u32>::new()
                .root(root_id, reg)
                .then(store, reg)
                .build(),
        );
        boxed.run(&mut world, 77u32);
        assert_eq!(*world.resource::<u64>(), 77);
    }

    // -- World access (Res<T>, ResMut<T>) in nodes --

    #[test]
    fn dag_world_access() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10); // factor
        wb.register::<String>(String::new());
        let mut world = wb.build();
        let reg = world.registry();

        fn scale(factor: Res<u64>, val: &u32) -> u64 {
            *factor * (*val as u64)
        }
        fn store(mut out: ResMut<String>, val: &u64) {
            *out = val.to_string();
        }

        let mut dag = DagStart::<u32>::new()
            .root(|x: u32| x, reg)
            .then(scale, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 7u32);
        assert_eq!(world.resource::<String>().as_str(), "70");
    }

    // -- Root-only (terminal root outputting ()) --

    #[test]
    fn dag_root_only() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(
                |mut out: ResMut<u64>, x: u32| {
                    *out = x as u64;
                },
                reg,
            )
            .build();

        dag.run(&mut world, 42u32);
        assert_eq!(*world.resource::<u64>(), 42);
    }

    // -- Multiple dispatches reuse state --

    #[test]
    fn dag_multiple_dispatches() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        fn root_id(x: u32) -> u64 {
            x as u64
        }
        fn store(mut out: ResMut<u64>, val: &u64) {
            *out = *val;
        }

        let mut dag = DagStart::<u32>::new()
            .root(root_id, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 1u32);
        assert_eq!(*world.resource::<u64>(), 1);
        dag.run(&mut world, 2u32);
        assert_eq!(*world.resource::<u64>(), 2);
        dag.run(&mut world, 3u32);
        assert_eq!(*world.resource::<u64>(), 3);
    }

    // -- 3-way merge --

    #[test]
    fn dag_3way_merge() {
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();
        let reg = world.registry();

        fn root_id(x: u32) -> u64 {
            x as u64
        }
        fn mul1(val: &u64) -> u64 {
            *val
        }
        fn mul2(val: &u64) -> u64 {
            *val * 2
        }
        fn mul3(val: &u64) -> u64 {
            *val * 3
        }
        fn merge3_fmt(mut out: ResMut<String>, a: &u64, b: &u64, c: &u64) {
            *out = format!("{},{},{}", a, b, c);
        }

        let mut dag = DagStart::<u32>::new()
            .root(root_id, reg)
            .fork()
            .arm(|a| a.then(mul1, reg))
            .arm(|b| b.then(mul2, reg))
            .arm(|c| c.then(mul3, reg))
            .merge(merge3_fmt, reg)
            .build();

        dag.run(&mut world, 10u32);
        assert_eq!(world.resource::<String>().as_str(), "10,20,30");
    }

    // -- DAG combinators --

    #[test]
    fn dag_dispatch() {
        fn root(x: u32) -> u64 {
            x as u64 + 42
        }
        fn sink(mut out: ResMut<u64>, event: u64) {
            *out = event;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .dispatch(sink.into_handler(reg))
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 42);
    }

    #[test]
    fn dag_option_map() {
        fn root(_x: u32) -> Option<u64> {
            Some(10)
        }
        fn double(val: &u64) -> u64 {
            *val * 2
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .map(double, reg)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 20);
    }

    #[test]
    fn dag_option_map_none() {
        fn root(_x: u32) -> Option<u64> {
            None
        }
        fn double(val: &u64) -> u64 {
            *val * 2
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(999);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .map(double, reg)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 999);
    }

    #[test]
    fn dag_option_and_then() {
        fn root(_x: u32) -> Option<u64> {
            Some(5)
        }
        fn check(val: &u64) -> Option<u64> {
            if *val > 3 { Some(*val * 10) } else { None }
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .and_then(check, reg)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 50);
    }

    #[test]
    fn dag_option_filter_keeps() {
        fn root(_x: u32) -> Option<u64> {
            Some(5)
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        let mut dag = DagStart::<u32>::new()
            .root(root, world.registry())
            .filter(|_w, v: &u64| *v > 3)
            .then(sink, world.registry())
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 5);
    }

    #[test]
    fn dag_option_filter_drops() {
        fn root(_x: u32) -> Option<u64> {
            Some(5)
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        let mut dag = DagStart::<u32>::new()
            .root(root, world.registry())
            .filter(|_w, v: &u64| *v > 10)
            .then(sink, world.registry())
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 0);
    }

    #[test]
    fn dag_option_on_none() {
        fn root(_x: u32) -> Option<u64> {
            None
        }
        fn sink(_val: &Option<u64>) {}
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();
        let reg = world.registry();

        let flag_id = world.registry().id::<bool>();
        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .on_none(move |w: &mut World| {
                // SAFETY: flag_id is a valid ResourceId for bool.
                unsafe { *w.get_mut::<bool>(flag_id) = true };
            })
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert!(*world.resource::<bool>());
    }

    #[test]
    fn dag_option_unwrap_or() {
        fn root(_x: u32) -> Option<u64> {
            None
        }
        fn sink(mut out: ResMut<u64>, val: &u64) {
            *out = *val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .unwrap_or(42u64)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 42);
    }

    #[test]
    fn dag_option_ok_or() {
        fn root(_x: u32) -> Option<u64> {
            None
        }
        fn sink(mut out: ResMut<u64>, val: &Result<u64, &str>) {
            *out = match val {
                Ok(v) => *v,
                Err(_) => 999,
            };
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .ok_or("missing")
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 999);
    }

    #[test]
    fn dag_result_map() {
        fn root(_x: u32) -> Result<u64, &'static str> {
            Ok(10)
        }
        fn double(val: &u64) -> u64 {
            *val * 2
        }
        fn sink(mut out: ResMut<u64>, val: &Result<u64, &str>) {
            *out = val.as_ref().copied().unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .map(double, reg)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 20);
    }

    #[test]
    fn dag_result_and_then() {
        fn root(_x: u32) -> Result<u64, &'static str> {
            Ok(5)
        }
        fn check(val: &u64) -> Result<u64, &'static str> {
            if *val > 3 {
                Ok(*val * 10)
            } else {
                Err("too small")
            }
        }
        fn sink(mut out: ResMut<u64>, val: &Result<u64, &str>) {
            *out = val.as_ref().copied().unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .and_then(check, reg)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 50);
    }

    #[test]
    fn dag_result_catch() {
        fn root(_x: u32) -> Result<u64, String> {
            Err("oops".into())
        }
        fn handle_err(mut log: ResMut<String>, err: &String) {
            *log = err.clone();
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<String>(String::new());
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .catch(handle_err, reg)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 0);
        assert_eq!(world.resource::<String>().as_str(), "oops");
    }

    #[test]
    fn dag_result_ok() {
        fn root(_x: u32) -> Result<u64, &'static str> {
            Err("fail")
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .ok()
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 0);
    }

    #[test]
    fn dag_result_unwrap_or_else() {
        fn root(_x: u32) -> Result<u64, &'static str> {
            Err("fail")
        }
        fn sink(mut out: ResMut<u64>, val: &u64) {
            *out = *val;
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .unwrap_or_else(|_w, _err| 42u64)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 42);
    }

    #[test]
    fn dag_result_map_err() {
        fn root(_x: u32) -> Result<u64, u32> {
            Err(5)
        }
        fn sink(mut out: ResMut<u64>, val: &Result<u64, String>) {
            *out = match val {
                Ok(v) => *v,
                Err(e) => e.len() as u64,
            };
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .map_err(|_w, e: u32| format!("err:{e}"))
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        // "err:5".len() == 5
        assert_eq!(*world.resource::<u64>(), 5);
    }

    #[test]
    fn dag_arm_combinators() {
        fn root(x: u32) -> u64 {
            x as u64 + 10
        }
        fn arm_step(val: &u64) -> Option<u64> {
            if *val > 5 { Some(*val * 3) } else { None }
        }
        fn double(val: &u64) -> u64 {
            *val * 2
        }
        fn merge_fn(a: &u64, b: &u64) -> String {
            format!("{a},{b}")
        }
        fn sink(mut out: ResMut<String>, val: &String) {
            *out = val.clone();
        }
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();
        let reg = world.registry();

        // Arm 0: root → arm_step (Option) → unwrap_or(0)
        // Arm 1: root → double
        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .fork()
            .arm(|a| a.then(arm_step, reg).unwrap_or(0u64))
            .arm(|b| b.then(double, reg))
            .merge(merge_fn, reg)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        // root(0) = 10
        // arm0: 10 > 5 → Some(30) → unwrap → 30
        // arm1: 10 * 2 = 20
        assert_eq!(world.resource::<String>().as_str(), "30,20");
    }

    #[test]
    fn dag_option_inspect() {
        fn root(_x: u32) -> Option<u64> {
            Some(42)
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<bool>(false);
        let mut world = wb.build();
        let reg = world.registry();

        let flag_id = world.registry().id::<bool>();
        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .inspect(move |w: &mut World, _val: &u64| {
                // SAFETY: flag_id is a valid ResourceId for bool.
                unsafe { *w.get_mut::<bool>(flag_id) = true };
            })
            .then(sink, reg)
            .build();

        dag.run(&mut world, 0u32);
        assert_eq!(*world.resource::<u64>(), 42);
        assert!(*world.resource::<bool>());
    }

    // -- Guard combinator --

    #[test]
    fn dag_guard_keeps() {
        fn root(x: u32) -> u64 {
            x as u64
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(0);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .guard(|_w, v| *v > 3)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 5);
    }

    #[test]
    fn dag_guard_drops() {
        fn root(x: u32) -> u64 {
            x as u64
        }
        fn sink(mut out: ResMut<u64>, val: &Option<u64>) {
            *out = val.unwrap_or(999);
        }
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .guard(|_w, v| *v > 10)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 999);
    }

    #[test]
    fn dag_arm_guard() {
        fn root(x: u32) -> u64 {
            x as u64
        }
        fn double(val: &u64) -> u64 {
            *val * 2
        }
        fn merge_fn(a: &Option<u64>, b: &u64) -> String {
            format!("{:?},{}", a, b)
        }
        fn sink(mut out: ResMut<String>, val: &String) {
            *out = val.clone();
        }
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();
        let reg = world.registry();

        // arm_a: guard drops (5 < 10), arm_b: runs normally
        let mut dag = DagStart::<u32>::new()
            .root(root, reg)
            .fork()
            .arm(|a| a.then(double, reg).guard(|_w, v| *v > 100))
            .arm(|b| b.then(double, reg))
            .merge(merge_fn, reg)
            .then(sink, reg)
            .build();

        dag.run(&mut world, 5u32);
        // arm_a: 10, guard fails → None. arm_b: 10.
        assert_eq!(world.resource::<String>().as_str(), "None,10");
    }
}
