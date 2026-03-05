// Builder return types use complex generics for compile-time edge validation.
#![allow(clippy::type_complexity)]

//! DAG pipeline — typed data-flow graphs with fan-out and merge.
//!
//! [`DagBuilder`] declares stages and edges with compile-time type validation.
//! [`DagPipeline`] executes the topologically sorted plan with pre-allocated
//! slots — zero allocation after build. Implements [`Handler<E>`] for
//! composition with the rest of nexus-rt.
//!
//! Stages receive their input **by reference** — data lives in pre-allocated
//! slots and downstream stages borrow from them. Fan-out is free (multiple
//! stages borrow the same slot). Stages produce owned output values that
//! are written into their output slot.
//!
//! # When to use
//!
//! Use DAG pipelines when data needs to fan out to multiple stages and
//! merge back. For linear chains, prefer [`PipelineStart`](crate::PipelineStart)
//! (monomorphized, zero vtable calls). For dynamic fan-out by reference,
//! use [`FanOut`](crate::FanOut) or [`Broadcast`](crate::Broadcast).
//!
//! # Stage signatures
//!
//! The root stage takes the event by value (passed directly from the stack).
//! All other stages take their input by reference:
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
//! use nexus_rt::dag::DagBuilder;
//!
//! let mut wb = WorldBuilder::new();
//! wb.register::<u64>(0);
//! let mut world = wb.build();
//! let reg = world.registry();
//!
//! let mut dag = DagBuilder::<u32>::new();
//! let root = dag.root(|x: u32| x as u64 * 2, reg);
//! let sink = dag.stage(
//!     |mut out: ResMut<u64>, val: &u64| { *out = *val; },
//!     reg,
//! );
//! dag.edge(root, sink);
//!
//! let mut pipeline = dag.build();
//! pipeline.run(&mut world, 5u32);
//! assert_eq!(*world.resource::<u64>(), 10);
//! ```

use std::alloc::{self, Layout};
use std::any::TypeId;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ptr::{self, NonNull};

use crate::Handler;
use crate::pipeline::{IntoStage, StageCall};
use crate::world::{Registry, World};

// =============================================================================
// DagStageId — typed handle for compile-time edge validation
// =============================================================================

/// Typed handle identifying a stage in a [`DagBuilder`].
///
/// The type parameters encode the stage's input/output types, enabling
/// compile-time validation of edges via [`DagBuilder::edge`]. `In` and
/// `Out` are **value types** (not references) — the DAG builder handles
/// the by-reference dispatch internally.
///
/// Returned by [`DagBuilder::root`], [`DagBuilder::stage`], and
/// `DagBuilder::merge*` methods.
pub struct DagStageId<In, Out> {
    idx: usize,
    _marker: PhantomData<fn(In) -> Out>,
}

// Manual impls to avoid bounds on In/Out.
impl<In, Out> Clone for DagStageId<In, Out> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<In, Out> Copy for DagStageId<In, Out> {}

// =============================================================================
// ErasedDagStage — type-erased stage dispatch
// =============================================================================

/// Type-erased stage dispatch.
///
/// # Safety
///
/// Implementors must ensure `run_erased` reads inputs and writes output
/// of the correct types — the concrete types known at stage declaration time.
trait ErasedDagStage: Send {
    /// Run the stage.
    ///
    /// # Safety
    /// - `inputs` pointers must point to valid, initialized values of the
    ///   correct types. Values are borrowed, not consumed.
    /// - `output` must point to allocated memory with correct layout.
    ///   The stage writes its result here via `ptr::write`.
    unsafe fn run_erased(&mut self, world: &mut World, inputs: &[*const u8], output: *mut u8);
}

// =============================================================================
// SingleInputStage — wraps a StageCall for single-input stages
// =============================================================================

/// Wraps a resolved `StageCall<&In, Out>` for type-erased dispatch.
///
/// The stage receives its input by reference — the slot owns the value,
/// the stage borrows it.
struct SingleInputStage<S, In, Out> {
    stage: S,
    _marker: PhantomData<fn(*const In) -> Out>,
}

impl<S, In, Out> ErasedDagStage for SingleInputStage<S, In, Out>
where
    S: for<'a> StageCall<&'a In, Out> + Send,
    In: 'static,
    Out: 'static,
{
    unsafe fn run_erased(&mut self, world: &mut World, inputs: &[*const u8], output: *mut u8) {
        debug_assert_eq!(inputs.len(), 1);
        // SAFETY: caller guarantees inputs[0] points to a valid, initialized In.
        // We borrow it — the slot retains ownership.
        let input: &In = unsafe { &*(inputs[0] as *const In) };
        let result = self.stage.call(world, input);
        // SAFETY: caller guarantees output points to memory with correct layout.
        unsafe { ptr::write(output as *mut Out, result) };
    }
}

/// Wraps a resolved `StageCall<In, Out>` for the root stage.
///
/// The root stage receives the event by value — moved from the event
/// slot via `ptr::read`.
struct RootStage<S, In, Out> {
    stage: S,
    _marker: PhantomData<fn(In) -> Out>,
}

impl<S, In, Out> ErasedDagStage for RootStage<S, In, Out>
where
    S: StageCall<In, Out> + Send,
    In: 'static,
    Out: 'static,
{
    unsafe fn run_erased(&mut self, world: &mut World, inputs: &[*const u8], output: *mut u8) {
        debug_assert_eq!(inputs.len(), 1);
        // SAFETY: caller guarantees inputs[0] points to a valid In.
        // Root consumes the event — it's only read once.
        let input = unsafe { ptr::read(inputs[0] as *const In) };
        let result = self.stage.call(world, input);
        // SAFETY: caller guarantees output has correct layout.
        unsafe { ptr::write(output as *mut Out, result) };
    }
}

// =============================================================================
// MergeStageCall / IntoMergeStage — merge stage dispatch
// =============================================================================

/// Callable trait for resolved merge stages.
///
/// Like [`StageCall`] but for merge stages with multiple reference inputs
/// bundled as `Inputs` (e.g. `(&'a A, &'a B)`).
#[doc(hidden)]
pub trait MergeStageCall<Inputs, Out> {
    /// Call this merge stage with a world reference and input references.
    fn call(&mut self, world: &mut World, inputs: Inputs) -> Out;
}

/// Converts a named function into a resolved merge stage.
///
/// Params first, then N reference inputs, returns output:
///
/// ```ignore
/// fn check(config: Res<Config>, ob: &ObResult, risk: &RiskResult) -> Decision { .. }
/// ```
#[doc(hidden)]
pub trait IntoMergeStage<Inputs, Out, Params> {
    /// The concrete resolved merge stage type.
    type Stage: MergeStageCall<Inputs, Out>;

    /// Resolve Param state from the registry and produce a merge stage.
    fn into_merge_stage(self, registry: &Registry) -> Self::Stage;
}

/// Internal: pre-resolved merge stage with cached Param state.
#[doc(hidden)]
pub struct MergeStage<F, Params: crate::handler::Param> {
    f: F,
    state: Params::State,
    #[allow(dead_code)]
    name: &'static str,
}

// -- Merge arity 2 -----------------------------------------------------------

// Param arity 0: closures work.
impl<A, B, Out, F> MergeStageCall<(&A, &B), Out> for MergeStage<F, ()>
where
    F: FnMut(&A, &B) -> Out + 'static,
{
    #[inline(always)]
    fn call(&mut self, _world: &mut World, inputs: (&A, &B)) -> Out {
        (self.f)(inputs.0, inputs.1)
    }
}

impl<A, B, Out, F> IntoMergeStage<(&A, &B), Out, ()> for F
where
    F: FnMut(&A, &B) -> Out + 'static,
{
    type Stage = MergeStage<F, ()>;

    fn into_merge_stage(self, registry: &Registry) -> Self::Stage {
        MergeStage {
            f: self,
            state: <() as crate::handler::Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

// Param arities 1-8 for merge arity 2.
macro_rules! impl_merge2_stage {
    ($($P:ident),+) => {
        impl<A, B, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            MergeStageCall<(&A, &B), Out> for MergeStage<F, ($($P,)+)>
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
            IntoMergeStage<(&A, &B), Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &A, &B) -> Out +
                FnMut($($P::Item<'a>,)+ &A, &B) -> Out,
        {
            type Stage = MergeStage<F, ($($P,)+)>;

            fn into_merge_stage(self, registry: &Registry) -> Self::Stage {
                let state = <($($P,)+) as crate::handler::Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $((<$P as crate::handler::Param>::resource_id($P),
                           std::any::type_name::<$P>()),)+
                    ]);
                }
                MergeStage { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

// ErasedDagStage wrapper for merge-2.
struct MergeStageWrapper2<S, A, B, Out> {
    stage: S,
    _marker: PhantomData<fn(*const A, *const B) -> Out>,
}

impl<S, A, B, Out> ErasedDagStage for MergeStageWrapper2<S, A, B, Out>
where
    S: for<'a> MergeStageCall<(&'a A, &'a B), Out> + Send,
    A: 'static,
    B: 'static,
    Out: 'static,
{
    unsafe fn run_erased(&mut self, world: &mut World, inputs: &[*const u8], output: *mut u8) {
        debug_assert_eq!(inputs.len(), 2);
        let a: &A = unsafe { &*(inputs[0] as *const A) };
        let b: &B = unsafe { &*(inputs[1] as *const B) };
        let result = self.stage.call(world, (a, b));
        unsafe { ptr::write(output as *mut Out, result) };
    }
}

// -- Merge arity 3 -----------------------------------------------------------

impl<A, B, C, Out, F> MergeStageCall<(&A, &B, &C), Out> for MergeStage<F, ()>
where
    F: FnMut(&A, &B, &C) -> Out + 'static,
{
    #[inline(always)]
    fn call(&mut self, _world: &mut World, inputs: (&A, &B, &C)) -> Out {
        (self.f)(inputs.0, inputs.1, inputs.2)
    }
}

impl<A, B, C, Out, F> IntoMergeStage<(&A, &B, &C), Out, ()> for F
where
    F: FnMut(&A, &B, &C) -> Out + 'static,
{
    type Stage = MergeStage<F, ()>;

    fn into_merge_stage(self, registry: &Registry) -> Self::Stage {
        MergeStage {
            f: self,
            state: <() as crate::handler::Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

macro_rules! impl_merge3_stage {
    ($($P:ident),+) => {
        impl<A, B, C, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            MergeStageCall<(&A, &B, &C), Out> for MergeStage<F, ($($P,)+)>
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
            IntoMergeStage<(&A, &B, &C), Out, ($($P,)+)> for F
        where
            for<'a> &'a mut F:
                FnMut($($P,)+ &A, &B, &C) -> Out +
                FnMut($($P::Item<'a>,)+ &A, &B, &C) -> Out,
        {
            type Stage = MergeStage<F, ($($P,)+)>;

            fn into_merge_stage(self, registry: &Registry) -> Self::Stage {
                let state = <($($P,)+) as crate::handler::Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $((<$P as crate::handler::Param>::resource_id($P),
                           std::any::type_name::<$P>()),)+
                    ]);
                }
                MergeStage { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

struct MergeStageWrapper3<S, A, B, C, Out> {
    stage: S,
    _marker: PhantomData<fn(*const A, *const B, *const C) -> Out>,
}

impl<S, A, B, C, Out> ErasedDagStage for MergeStageWrapper3<S, A, B, C, Out>
where
    S: for<'a> MergeStageCall<(&'a A, &'a B, &'a C), Out> + Send,
    A: 'static,
    B: 'static,
    C: 'static,
    Out: 'static,
{
    unsafe fn run_erased(&mut self, world: &mut World, inputs: &[*const u8], output: *mut u8) {
        debug_assert_eq!(inputs.len(), 3);
        let a: &A = unsafe { &*(inputs[0] as *const A) };
        let b: &B = unsafe { &*(inputs[1] as *const B) };
        let c: &C = unsafe { &*(inputs[2] as *const C) };
        let result = self.stage.call(world, (a, b, c));
        unsafe { ptr::write(output as *mut Out, result) };
    }
}

// -- Merge arity 4 -----------------------------------------------------------

impl<A, B, C, D, Out, F> MergeStageCall<(&A, &B, &C, &D), Out> for MergeStage<F, ()>
where
    F: FnMut(&A, &B, &C, &D) -> Out + 'static,
{
    #[inline(always)]
    fn call(&mut self, _world: &mut World, i: (&A, &B, &C, &D)) -> Out {
        (self.f)(i.0, i.1, i.2, i.3)
    }
}

impl<A, B, C, D, Out, F> IntoMergeStage<(&A, &B, &C, &D), Out, ()> for F
where
    F: FnMut(&A, &B, &C, &D) -> Out + 'static,
{
    type Stage = MergeStage<F, ()>;
    fn into_merge_stage(self, registry: &Registry) -> Self::Stage {
        MergeStage {
            f: self,
            state: <() as crate::handler::Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

macro_rules! impl_merge4_stage {
    ($($P:ident),+) => {
        impl<A, B, C, D, Out, F: 'static, $($P: crate::handler::Param + 'static),+>
            MergeStageCall<(&A, &B, &C, &D), Out> for MergeStage<F, ($($P,)+)>
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
            IntoMergeStage<(&A, &B, &C, &D), Out, ($($P,)+)> for F
        where for<'a> &'a mut F:
            FnMut($($P,)+ &A, &B, &C, &D) -> Out +
            FnMut($($P::Item<'a>,)+ &A, &B, &C, &D) -> Out,
        {
            type Stage = MergeStage<F, ($($P,)+)>;
            fn into_merge_stage(self, registry: &Registry) -> Self::Stage {
                let state = <($($P,)+) as crate::handler::Param>::init(registry);
                { #[allow(non_snake_case)] let ($($P,)+) = &state;
                  registry.check_access(&[$((<$P as crate::handler::Param>::resource_id($P), std::any::type_name::<$P>()),)+]); }
                MergeStage { f: self, state, name: std::any::type_name::<F>() }
            }
        }
    };
}

struct MergeStageWrapper4<S, A, B, C, D, Out> {
    stage: S,
    _marker: PhantomData<fn(*const A, *const B, *const C, *const D) -> Out>,
}
impl<S, A: 'static, B: 'static, C: 'static, D: 'static, Out: 'static> ErasedDagStage
    for MergeStageWrapper4<S, A, B, C, D, Out>
where
    S: for<'a> MergeStageCall<(&'a A, &'a B, &'a C, &'a D), Out> + Send,
{
    unsafe fn run_erased(&mut self, world: &mut World, inputs: &[*const u8], output: *mut u8) {
        debug_assert_eq!(inputs.len(), 4);
        let (a, b, c, d) = unsafe {
            (
                &*(inputs[0] as *const A),
                &*(inputs[1] as *const B),
                &*(inputs[2] as *const C),
                &*(inputs[3] as *const D),
            )
        };
        let result = self.stage.call(world, (a, b, c, d));
        unsafe { ptr::write(output as *mut Out, result) };
    }
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

all_tuples!(impl_merge2_stage);
all_tuples!(impl_merge3_stage);
all_tuples!(impl_merge4_stage);

// =============================================================================
// StageMeta — per-stage metadata captured at declaration time
// =============================================================================

struct StageMeta {
    name: &'static str,
    output_type_id: TypeId,
    output_layout: Layout,
    drop_fn: unsafe fn(*mut u8),
    /// For merge stages: ordered upstream stage indices.
    merge_inputs: Option<Vec<usize>>,
}

/// Monomorphized drop-in-place.
///
/// # Safety
/// - `ptr` must point to a valid, initialized `T`
/// - Must only be called once per value
unsafe fn drop_value<T>(ptr: *mut u8) {
    // SAFETY: caller guarantees ptr points to valid T, called once.
    unsafe { ptr::drop_in_place(ptr as *mut T) };
}

// =============================================================================
// SlotStorage — pre-allocated value slots
// =============================================================================

/// Pre-allocated memory slots for inter-stage data flow.
///
/// Each slot is heap-allocated at build time and reused across dispatches.
/// Stages write output via `ptr::write`, downstream stages borrow via
/// `&*ptr` — no cloning for fan-out.
struct SlotStorage {
    ptrs: Vec<*mut u8>,
    layouts: Vec<Layout>,
    drop_fns: Vec<unsafe fn(*mut u8)>,
}

impl SlotStorage {
    fn new() -> Self {
        Self {
            ptrs: Vec::new(),
            layouts: Vec::new(),
            drop_fns: Vec::new(),
        }
    }

    /// Allocate a new slot. Returns the slot index.
    fn alloc_slot(&mut self, layout: Layout, drop_fn: unsafe fn(*mut u8)) -> usize {
        let idx = self.ptrs.len();
        let ptr = if layout.size() == 0 {
            NonNull::dangling().as_ptr()
        } else {
            // SAFETY: layout.size() > 0, layout is valid (from Layout::new::<T>()).
            let ptr = unsafe { alloc::alloc(layout) };
            if ptr.is_null() {
                alloc::handle_alloc_error(layout);
            }
            ptr
        };
        self.ptrs.push(ptr);
        self.layouts.push(layout);
        self.drop_fns.push(drop_fn);
        idx
    }
}

impl Drop for SlotStorage {
    fn drop(&mut self) {
        for (ptr, layout) in self.ptrs.iter().zip(self.layouts.iter()) {
            if layout.size() > 0 {
                // SAFETY: ptr was allocated with this layout in alloc_slot.
                // Values have been dropped via the occupied bitmap before
                // SlotStorage::drop runs.
                unsafe { alloc::dealloc(*ptr, *layout) };
            }
        }
    }
}

// SAFETY: SlotStorage contains raw pointers to heap allocations it owns.
// The pointers don't alias external memory — they point to private heap
// blocks allocated in alloc_slot(). Moving SlotStorage across threads is
// safe because the heap allocations move with it (pointer stability).
unsafe impl Send for SlotStorage {}

// =============================================================================
// ExecutionStep — flat execution plan
// =============================================================================

/// A single step in the flattened execution plan.
///
/// All pointers are pre-resolved at build time from the slot arena.
/// At dispatch time, the pipeline walks these in order — no graph
/// traversal, no pointer resolution, no allocation.
struct ExecutionStep {
    stage_idx: usize,
    /// Pre-resolved input pointers into the slot arena.
    input_ptrs: Vec<*const u8>,
    /// Pre-resolved output pointer into the slot arena.
    output_ptr: *mut u8,
    /// Bit index for the occupied bitmap.
    output_bit: usize,
}

// SAFETY: ExecutionStep contains raw pointers into the SlotStorage arena.
// These are stable heap addresses owned by the parent DagPipeline.
// ExecutionStep never outlives its arena.
unsafe impl Send for ExecutionStep {}

// =============================================================================
// DagBuilder — cold-path builder
// =============================================================================

/// Builder for declaring DAG pipeline stages and edges.
///
/// Stages are declared with [`root`](Self::root), [`stage`](Self::stage),
/// and `merge*` methods. Edges connect stages with compile-time type
/// validation via [`edge`](Self::edge). Call [`build`](Self::build) to
/// produce a [`DagPipeline`] that implements [`Handler<E>`].
///
/// The root stage receives the event by value. All other stages receive
/// their input by reference from pre-allocated slots. Fan-out is free —
/// multiple downstream stages borrow the same slot.
///
/// # Panics
///
/// [`build`](Self::build) panics if:
/// - No root stage declared
/// - Cycle detected
/// - Unreachable stages exist
/// - Terminal stages (out-degree 0) produce non-`()` output
pub struct DagBuilder<E> {
    stages: Vec<Box<dyn ErasedDagStage>>,
    edges: Vec<(usize, usize)>,
    stage_meta: Vec<StageMeta>,
    root_idx: Option<usize>,
    _marker: PhantomData<fn(E)>,
}

impl<E: 'static> Default for DagBuilder<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: 'static> DagBuilder<E> {
    /// Create a new DAG builder.
    pub fn new() -> Self {
        Self {
            stages: Vec::new(),
            edges: Vec::new(),
            stage_meta: Vec::new(),
            root_idx: None,
            _marker: PhantomData,
        }
    }

    /// Declare the root stage. Takes the pipeline event `E` by value,
    /// produces `Out`.
    ///
    /// # Panics
    ///
    /// Panics if called more than once.
    pub fn root<Out, Params, S>(&mut self, f: S, registry: &Registry) -> DagStageId<E, Out>
    where
        Out: 'static,
        S: IntoStage<E, Out, Params>,
        S::Stage: Send + 'static,
    {
        assert!(
            self.root_idx.is_none(),
            "DagBuilder: root() called twice — only one root stage allowed"
        );
        let idx = self.stages.len();
        assert!(idx < 64, "DagBuilder: maximum 64 stages");

        let resolved = f.into_stage(registry);
        let name = std::any::type_name::<S>();

        self.stages.push(Box::new(RootStage::<_, E, Out> {
            stage: resolved,
            _marker: PhantomData,
        }));

        self.stage_meta.push(StageMeta {
            name,
            output_type_id: TypeId::of::<Out>(),
            output_layout: Layout::new::<Out>(),
            drop_fn: drop_value::<Out>,
            merge_inputs: None,
        });

        self.root_idx = Some(idx);
        DagStageId {
            idx,
            _marker: PhantomData,
        }
    }

    /// Declare a regular stage. Takes `&In` by reference, produces `Out`.
    ///
    /// The stage function signature should be:
    /// ```ignore
    /// fn my_stage(params: Res<T>, ..., input: &In) -> Out { .. }
    /// ```
    pub fn stage<In, Out, Params, S>(&mut self, f: S, registry: &Registry) -> DagStageId<In, Out>
    where
        In: 'static,
        Out: 'static,
        // IntoStage with a concrete (elided) reference. The resolved
        // Stage must be callable for any lifetime via StageCall<&In, Out>.
        S: IntoStage<&'static In, Out, Params>,
        S::Stage: for<'a> StageCall<&'a In, Out> + Send + 'static,
    {
        let idx = self.stages.len();
        assert!(idx < 64, "DagBuilder: maximum 64 stages");

        // SAFETY: we use 'static as a placeholder for IntoStage resolution.
        // The Stage bound `for<'a> StageCall<&'a In, Out>` ensures it
        // actually works for any lifetime at dispatch time.
        let resolved = f.into_stage(registry);
        let name = std::any::type_name::<S>();

        self.stages.push(Box::new(SingleInputStage::<_, In, Out> {
            stage: resolved,
            _marker: PhantomData,
        }));

        self.stage_meta.push(StageMeta {
            name,
            output_type_id: TypeId::of::<Out>(),
            output_layout: Layout::new::<Out>(),
            drop_fn: drop_value::<Out>,
            merge_inputs: None,
        });

        DagStageId {
            idx,
            _marker: PhantomData,
        }
    }

    /// Connect two stages. Compile-time type validation: `from`'s output
    /// type must match `to`'s input type.
    pub fn edge<A, B, C>(&mut self, from: DagStageId<A, B>, to: DagStageId<B, C>) {
        self.edges.push((from.idx, to.idx));
    }

    /// Merge two upstream stages into one. Creates edges automatically.
    ///
    /// The merge function receives both upstream outputs by reference:
    ///
    /// ```ignore
    /// fn check(config: Res<Config>, ob: &ObResult, risk: &RiskResult) -> Decision { .. }
    /// ```
    ///
    /// Returns a `DagStageId<(A, B), Out>` — the input type is a tuple
    /// for edge validation purposes, but the function takes separate
    /// reference arguments.
    pub fn merge2<X, A, Y, B, Out, Params, S>(
        &mut self,
        a: DagStageId<X, A>,
        b: DagStageId<Y, B>,
        f: S,
        registry: &Registry,
    ) -> DagStageId<(A, B), Out>
    where
        A: 'static,
        B: 'static,
        Out: 'static,
        S: IntoMergeStage<(&'static A, &'static B), Out, Params>,
        S::Stage: for<'x> MergeStageCall<(&'x A, &'x B), Out> + Send + 'static,
    {
        let idx = self.stages.len();
        assert!(idx < 64, "DagBuilder: maximum 64 stages");

        let resolved = f.into_merge_stage(registry);
        let name = std::any::type_name::<S>();

        self.stages
            .push(Box::new(MergeStageWrapper2::<_, A, B, Out> {
                stage: resolved,
                _marker: PhantomData,
            }));

        let merge_inputs = vec![a.idx, b.idx];

        self.stage_meta.push(StageMeta {
            name,
            output_type_id: TypeId::of::<Out>(),
            output_layout: Layout::new::<Out>(),
            drop_fn: drop_value::<Out>,
            merge_inputs: Some(merge_inputs),
        });

        // Create edges automatically
        self.edges.push((a.idx, idx));
        self.edges.push((b.idx, idx));

        DagStageId {
            idx,
            _marker: PhantomData,
        }
    }

    /// Merge three upstream stages into one. Creates edges automatically.
    pub fn merge3<X, A, Y, B, Z, C, Out, Params, S>(
        &mut self,
        a: DagStageId<X, A>,
        b: DagStageId<Y, B>,
        c: DagStageId<Z, C>,
        f: S,
        registry: &Registry,
    ) -> DagStageId<(A, B, C), Out>
    where
        A: 'static,
        B: 'static,
        C: 'static,
        Out: 'static,
        S: IntoMergeStage<(&'static A, &'static B, &'static C), Out, Params>,
        S::Stage: for<'x> MergeStageCall<(&'x A, &'x B, &'x C), Out> + Send + 'static,
    {
        let idx = self.stages.len();
        assert!(idx < 64, "DagBuilder: maximum 64 stages");

        let resolved = f.into_merge_stage(registry);
        let name = std::any::type_name::<S>();

        self.stages
            .push(Box::new(MergeStageWrapper3::<_, A, B, C, Out> {
                stage: resolved,
                _marker: PhantomData,
            }));

        let merge_inputs = vec![a.idx, b.idx, c.idx];

        self.stage_meta.push(StageMeta {
            name,
            output_type_id: TypeId::of::<Out>(),
            output_layout: Layout::new::<Out>(),
            drop_fn: drop_value::<Out>,
            merge_inputs: Some(merge_inputs),
        });

        self.edges.push((a.idx, idx));
        self.edges.push((b.idx, idx));
        self.edges.push((c.idx, idx));

        DagStageId {
            idx,
            _marker: PhantomData,
        }
    }

    /// Merge four upstream stages into one. Creates edges automatically.
    #[allow(clippy::many_single_char_names)]
    pub fn merge4<X1, A, X2, B, X3, C, X4, D, Out, Params, S>(
        &mut self,
        a: DagStageId<X1, A>,
        b: DagStageId<X2, B>,
        c: DagStageId<X3, C>,
        d: DagStageId<X4, D>,
        f: S,
        registry: &Registry,
    ) -> DagStageId<(A, B, C, D), Out>
    where
        A: 'static,
        B: 'static,
        C: 'static,
        D: 'static,
        Out: 'static,
        S: IntoMergeStage<(&'static A, &'static B, &'static C, &'static D), Out, Params>,
        S::Stage: for<'x> MergeStageCall<(&'x A, &'x B, &'x C, &'x D), Out> + Send + 'static,
    {
        let idx = self.stages.len();
        assert!(idx < 64, "DagBuilder: maximum 64 stages");
        let resolved = f.into_merge_stage(registry);
        let name = std::any::type_name::<S>();
        self.stages
            .push(Box::new(MergeStageWrapper4::<_, A, B, C, D, Out> {
                stage: resolved,
                _marker: PhantomData,
            }));
        self.stage_meta.push(StageMeta {
            name,
            output_type_id: TypeId::of::<Out>(),
            output_layout: Layout::new::<Out>(),
            drop_fn: drop_value::<Out>,
            merge_inputs: Some(vec![a.idx, b.idx, c.idx, d.idx]),
        });
        self.edges.push((a.idx, idx));
        self.edges.push((b.idx, idx));
        self.edges.push((c.idx, idx));
        self.edges.push((d.idx, idx));
        DagStageId {
            idx,
            _marker: PhantomData,
        }
    }

    /// Build the DAG into a [`DagPipeline`].
    ///
    /// Performs topological sort, validates the graph, and generates a
    /// flat execution plan with pre-allocated slots.
    ///
    /// # Panics
    ///
    /// - No root stage declared
    /// - Cycle detected in stage graph
    /// - Unreachable stages (not connected to root)
    /// - Terminal stages with non-`()` output
    /// - Non-root, non-merge stages with != 1 incoming edge
    pub fn build(self) -> DagPipeline<E> {
        let root_idx = self
            .root_idx
            .expect("DagBuilder: no root stage — call root() before build()");

        let n = self.stages.len();

        // -- adjacency + in-degree --
        let mut successors: Vec<Vec<usize>> = vec![vec![]; n];
        let mut in_degree: Vec<usize> = vec![0; n];
        for &(from, to) in &self.edges {
            successors[from].push(to);
            in_degree[to] += 1;
        }

        // -- Kahn's algorithm: topological sort --
        let mut queue = VecDeque::new();
        for i in 0..n {
            if in_degree[i] == 0 {
                queue.push_back(i);
            }
        }
        let mut topo_order = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            topo_order.push(node);
            for &succ in &successors[node] {
                in_degree[succ] -= 1;
                if in_degree[succ] == 0 {
                    queue.push_back(succ);
                }
            }
        }
        assert!(
            topo_order.len() == n,
            "DagBuilder: cycle detected in stage graph"
        );

        // -- connectivity: all stages reachable from root --
        let mut reachable = vec![false; n];
        reachable[root_idx] = true;
        for &node in &topo_order {
            if reachable[node] {
                for &succ in &successors[node] {
                    reachable[succ] = true;
                }
            }
        }
        for i in 0..n {
            assert!(
                reachable[i],
                "DagBuilder: stage {} ('{}') is unreachable from root",
                i, self.stage_meta[i].name
            );
        }

        // -- terminal check: out-degree 0 must output () --
        for i in 0..n {
            assert!(
                !successors[i].is_empty()
                    || self.stage_meta[i].output_type_id == TypeId::of::<()>(),
                "DagBuilder: terminal stage {} ('{}') must output () — \
                 use a final stage that consumes the value",
                i,
                self.stage_meta[i].name
            );
        }

        // -- allocate slot arena (output slots only, no event slot) --
        let mut slots = SlotStorage::new();

        // One output slot per stage
        let mut stage_output_slot: Vec<usize> = Vec::with_capacity(n);
        for meta in &self.stage_meta {
            let slot = slots.alloc_slot(meta.output_layout, meta.drop_fn);
            stage_output_slot.push(slot);
        }

        // -- build predecessors for input resolution --
        let mut predecessors: Vec<Vec<usize>> = vec![vec![]; n];
        for &(from, to) in &self.edges {
            predecessors[to].push(from);
        }

        // -- separate root from execution steps --
        // Root receives the event directly from the stack (not the arena).
        // Its output goes into an arena slot like every other stage.
        let root_output_slot = stage_output_slot[root_idx];
        let root_output_ptr = slots.ptrs[root_output_slot];
        let root_output_bit = root_output_slot;

        // -- generate execution steps for non-root stages --
        let mut steps = Vec::new();
        for &node in &topo_order {
            if node == root_idx {
                continue;
            }

            let input_slot_indices =
                if let Some(ref merge_inputs) = self.stage_meta[node].merge_inputs {
                    // Merge stage: inputs from specified upstream stages in order
                    merge_inputs
                        .iter()
                        .map(|&upstream| stage_output_slot[upstream])
                        .collect()
                } else {
                    // Regular stage: single input from upstream
                    let preds = &predecessors[node];
                    assert_eq!(
                        preds.len(),
                        1,
                        "DagBuilder: stage {} ('{}') has {} incoming edges, expected 1 \
                         (use merge for multiple inputs)",
                        node,
                        self.stage_meta[node].name,
                        preds.len()
                    );
                    vec![stage_output_slot[preds[0]]]
                };

            let output_slot_idx = stage_output_slot[node];

            // Pre-resolve slot indices into arena pointers.
            // These pointers are stable — they point to heap allocations
            // that don't move for the lifetime of the SlotStorage.
            steps.push(ExecutionStep {
                stage_idx: node,
                input_ptrs: input_slot_indices
                    .iter()
                    .map(|&s| slots.ptrs[s].cast_const())
                    .collect(),
                output_ptr: slots.ptrs[output_slot_idx],
                output_bit: output_slot_idx,
            });
        }

        DagPipeline {
            stages: self.stages,
            slots,
            steps,
            root_stage_idx: root_idx,
            root_output_ptr,
            root_output_bit,
            occupied: 0,
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// DagPipeline — hot-path runtime
// =============================================================================

/// Compiled DAG pipeline implementing [`Handler<E>`].
///
/// Created by [`DagBuilder::build`]. Executes a pre-computed topological
/// plan over a pre-allocated slot arena. All pointers are resolved at
/// build time — dispatch is zero-allocation.
///
/// Stages borrow their inputs from arena slots — fan-out is free
/// (multiple stages read the same slot by reference). The root stage
/// receives the event directly from the stack via [`ManuallyDrop`] —
/// it never enters the arena.
pub struct DagPipeline<E> {
    stages: Vec<Box<dyn ErasedDagStage>>,
    slots: SlotStorage,
    /// Execution steps for non-root stages (root handled separately).
    steps: Vec<ExecutionStep>,
    /// Index of the root stage in `stages`.
    root_stage_idx: usize,
    /// Pre-resolved output pointer for the root stage.
    root_output_ptr: *mut u8,
    /// Bit index for the root's output slot in the occupied bitmap.
    root_output_bit: usize,
    /// Bitmap tracking which slots contain initialized values.
    /// Used for drop safety if a stage panics mid-dispatch.
    occupied: u64,
    _marker: PhantomData<fn(E)>,
}

impl<E: 'static> Handler<E> for DagPipeline<E> {
    fn run(&mut self, world: &mut World, event: E) {
        // Root stage: pass event directly from the stack.
        // ManuallyDrop suppresses Rust's implicit drop — the event is consumed
        // by ptr::read inside RootStage::run_erased.
        let event = ManuallyDrop::new(event);
        let event_ptr: *const u8 = (&raw const *event) as *const u8;

        // SAFETY: RootStage::run_erased does ptr::read on inputs[0], consuming
        // the event bytes. ManuallyDrop prevents the caller from double-dropping.
        // root_output_ptr is pre-resolved from the arena with correct layout.
        unsafe {
            self.stages[self.root_stage_idx].run_erased(world, &[event_ptr], self.root_output_ptr);
        }
        self.occupied |= 1u64 << self.root_output_bit;

        // Remaining stages borrow from arena slots.
        let steps = &self.steps;
        let stages = &mut self.stages;
        let occupied = &mut self.occupied;

        for step in steps {
            // SAFETY:
            // - Input pointers are pre-resolved from the arena. Values are
            //   valid (written by root or prior steps). Stages borrow, not consume.
            // - Output pointer is pre-resolved from the arena with correct layout.
            // - stage_idx is valid (produced by build()).
            unsafe {
                stages[step.stage_idx].run_erased(world, &step.input_ptrs, step.output_ptr);
            }
            *occupied |= 1u64 << step.output_bit;
        }

        // Drop all stage output values.
        self.drop_occupied();
    }

    fn name(&self) -> &'static str {
        "DagPipeline"
    }
}

impl<E> DagPipeline<E> {
    /// Drop all values in occupied slots and clear the bitmap.
    fn drop_occupied(&mut self) {
        let mut bits = self.occupied;
        while bits != 0 {
            let slot = bits.trailing_zeros() as usize;
            if self.slots.layouts[slot].size() > 0 {
                // SAFETY: slot is occupied (bit set) and contains a valid value.
                // drop_fn was monomorphized for the correct type at build time.
                unsafe { (self.slots.drop_fns[slot])(self.slots.ptrs[slot]) };
            }
            bits &= bits - 1; // clear lowest set bit
        }
        self.occupied = 0;
    }
}

impl<E> Drop for DagPipeline<E> {
    fn drop(&mut self) {
        // Drop any values still in slots (e.g. if a stage panicked mid-dispatch).
        self.drop_occupied();
    }
}

// SAFETY: DagPipeline contains raw pointers into its owned SlotStorage arena.
// The arena heap allocations have stable addresses — they don't move when
// DagPipeline is moved across threads. No E values persist between runs
// (the event is consumed during dispatch, outputs are dropped at the end).
// All dispatch is single-threaded.
unsafe impl<E> Send for DagPipeline<E> {}

// =============================================================================
// Typed DAG — monomorphized, zero vtable dispatch
// =============================================================================
//
// Encodes DAG topology in the type system at compile time. After
// monomorphization, the entire DAG is a single flat function with all
// values as stack locals. No arena, no bitmap, no unsafe.
//
// Fan-out: multiple stages borrow the same stack local (no Clone).
// Merge: merge stage borrows all arm outputs.
// Panic safety: stack unwinding drops all locals automatically.

/// Entry point for building a typed (monomorphized) DAG pipeline.
///
/// The typed DAG encodes topology in the type system at compile time,
/// producing a single monomorphized closure chain. All values live as
/// stack locals in the `run()` body — no arena, no vtable dispatch,
/// no unsafe.
///
/// For dynamic topology (runtime edges), use [`DagBuilder`] instead.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, Handler};
/// use nexus_rt::dag::TypedDagStart;
///
/// let mut wb = WorldBuilder::new();
/// wb.register::<u64>(0);
/// let mut world = wb.build();
/// let reg = world.registry();
///
/// fn double(x: u32) -> u64 { x as u64 * 2 }
/// fn store(mut out: ResMut<u64>, val: &u64) { *out = *val; }
///
/// let mut dag = TypedDagStart::<u32>::new()
///     .root(double, reg)
///     .then(store, reg)
///     .build();
///
/// dag.run(&mut world, 5u32);
/// assert_eq!(*world.resource::<u64>(), 10);
/// ```
pub struct TypedDagStart<E>(PhantomData<fn(E)>);

impl<E: 'static> TypedDagStart<E> {
    /// Create a new typed DAG entry point.
    pub fn new() -> Self {
        Self(PhantomData)
    }

    /// Set the root stage. Takes the event `E` by value, produces `Out`.
    pub fn root<Out, Params, S>(
        self,
        f: S,
        registry: &Registry,
    ) -> TypedDagChain<E, Out, impl FnMut(&mut World, E) -> Out + use<E, Out, Params, S>>
    where
        Out: 'static,
        S: IntoStage<E, Out, Params>,
    {
        let mut resolved = f.into_stage(registry);
        TypedDagChain {
            chain: move |world: &mut World, event: E| resolved.call(world, event),
            _marker: PhantomData,
        }
    }
}

impl<E: 'static> Default for TypedDagStart<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Main chain builder for a typed DAG.
///
/// `Chain` is `FnMut(&mut World, E) -> Out` — the monomorphized closure
/// representing all stages composed so far.
pub struct TypedDagChain<E, Out, Chain> {
    chain: Chain,
    _marker: PhantomData<fn(E) -> Out>,
}

impl<E: 'static, Out: 'static, Chain> TypedDagChain<E, Out, Chain>
where
    Chain: FnMut(&mut World, E) -> Out,
{
    /// Append a stage. Takes `&Out` by reference, produces `NewOut`.
    pub fn then<NewOut, Params, S>(
        self,
        f: S,
        registry: &Registry,
    ) -> TypedDagChain<
        E,
        NewOut,
        impl FnMut(&mut World, E) -> NewOut + use<E, Out, NewOut, Params, Chain, S>,
    >
    where
        NewOut: 'static,
        S: IntoStage<&'static Out, NewOut, Params>,
        S::Stage: for<'a> StageCall<&'a Out, NewOut>,
    {
        let mut chain = self.chain;
        let mut resolved = f.into_stage(registry);
        TypedDagChain {
            chain: move |world: &mut World, event: E| {
                let out = chain(world, event);
                resolved.call(world, &out)
            },
            _marker: PhantomData,
        }
    }

    /// Enter fork mode. Subsequent `.arm()` calls add parallel branches.
    pub fn fork(self) -> TypedDagChainFork<E, Out, Chain, ()> {
        TypedDagChainFork {
            chain: self.chain,
            arms: (),
            _marker: PhantomData,
        }
    }
}

impl<E: 'static, Chain> TypedDagChain<E, (), Chain>
where
    Chain: FnMut(&mut World, E) + Send + 'static,
{
    /// Finalize into a [`TypedDag`] that implements [`Handler<E>`].
    ///
    /// Only available when the chain ends with `()`. If your DAG
    /// produces a value, add a final `.then()` that consumes the output.
    pub fn build(self) -> TypedDag<E, Chain> {
        TypedDag {
            chain: self.chain,
            _marker: PhantomData,
        }
    }
}

/// Arm builder seed. Passed to `.arm()` closures.
///
/// Call `.then()` to add the first stage in this arm.
pub struct TypedDagArmStart<In>(PhantomData<fn(*const In)>);

impl<In: 'static> TypedDagArmStart<In> {
    /// Add the first stage in this arm. Takes `&In` by reference.
    pub fn then<Out, Params, S>(
        self,
        f: S,
        registry: &Registry,
    ) -> TypedDagArm<In, Out, impl FnMut(&mut World, &In) -> Out + use<In, Out, Params, S>>
    where
        Out: 'static,
        S: IntoStage<&'static In, Out, Params>,
        S::Stage: for<'a> StageCall<&'a In, Out>,
    {
        let mut resolved = f.into_stage(registry);
        TypedDagArm {
            chain: move |world: &mut World, input: &In| resolved.call(world, input),
            _marker: PhantomData,
        }
    }
}

/// Built arm in a typed DAG fork.
///
/// `Chain` is `FnMut(&mut World, &In) -> Out` — the monomorphized
/// closure for this arm's stages.
pub struct TypedDagArm<In, Out, Chain> {
    chain: Chain,
    _marker: PhantomData<fn(*const In) -> Out>,
}

impl<In: 'static, Out: 'static, Chain> TypedDagArm<In, Out, Chain>
where
    Chain: FnMut(&mut World, &In) -> Out,
{
    /// Append a stage in this arm. Takes `&Out` by reference.
    pub fn then<NewOut, Params, S>(
        self,
        f: S,
        registry: &Registry,
    ) -> TypedDagArm<
        In,
        NewOut,
        impl FnMut(&mut World, &In) -> NewOut + use<In, Out, NewOut, Params, Chain, S>,
    >
    where
        NewOut: 'static,
        S: IntoStage<&'static Out, NewOut, Params>,
        S::Stage: for<'a> StageCall<&'a Out, NewOut>,
    {
        let mut chain = self.chain;
        let mut resolved = f.into_stage(registry);
        TypedDagArm {
            chain: move |world: &mut World, input: &In| {
                let out = chain(world, input);
                resolved.call(world, &out)
            },
            _marker: PhantomData,
        }
    }

    /// Enter fork mode within this arm.
    pub fn fork(self) -> TypedDagArmFork<In, Out, Chain, ()> {
        TypedDagArmFork {
            chain: self.chain,
            arms: (),
            _marker: PhantomData,
        }
    }
}

/// Fork builder on the main chain. Accumulates arms as a tuple.
pub struct TypedDagChainFork<E, ForkOut, Chain, Arms> {
    chain: Chain,
    arms: Arms,
    _marker: PhantomData<fn(E) -> ForkOut>,
}

/// Fork builder within an arm. Accumulates sub-arms as a tuple.
pub struct TypedDagArmFork<In, ForkOut, Chain, Arms> {
    chain: Chain,
    arms: Arms,
    _marker: PhantomData<fn(*const In) -> ForkOut>,
}

/// Final built typed DAG. Implements [`Handler<E>`].
///
/// Created by [`TypedDagChain::build`]. The entire DAG is monomorphized
/// at compile time — no boxing, no virtual dispatch, no arena.
pub struct TypedDag<E, Chain> {
    chain: Chain,
    _marker: PhantomData<fn(E)>,
}

impl<E: 'static, Chain> Handler<E> for TypedDag<E, Chain>
where
    Chain: FnMut(&mut World, E) + Send + 'static,
{
    fn run(&mut self, world: &mut World, event: E) {
        (self.chain)(world, event);
    }

    fn name(&self) -> &'static str {
        "TypedDag"
    }
}

// =============================================================================
// Fork arity macro — arm accumulation, merge, join
// =============================================================================

/// Generates arm accumulation, merge, and join for a fork type.
///
/// ChainFork and ArmFork differ only in:
/// - How the upstream chain is called (by value vs by reference)
/// - What output type is produced (TypedDagChain vs TypedDagArm)
macro_rules! impl_typed_dag_fork {
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
                f: impl FnOnce(TypedDagArmStart<ForkOut>) -> TypedDagArm<ForkOut, AOut, ACh>,
            ) -> $Fork<$U, ForkOut, Chain, (TypedDagArm<ForkOut, AOut, ACh>,)> {
                let arm = f(TypedDagArmStart(PhantomData));
                $Fork {
                    chain: self.chain,
                    arms: (arm,),
                    _marker: PhantomData,
                }
            }
        }

        impl<$U, ForkOut, Chain, A0, C0>
            $Fork<$U, ForkOut, Chain, (TypedDagArm<ForkOut, A0, C0>,)>
        {
            /// Add a second arm to this fork.
            pub fn arm<AOut, ACh>(
                self,
                f: impl FnOnce(TypedDagArmStart<ForkOut>) -> TypedDagArm<ForkOut, AOut, ACh>,
            ) -> $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    TypedDagArm<ForkOut, A0, C0>,
                    TypedDagArm<ForkOut, AOut, ACh>,
                ),
            > {
                let arm = f(TypedDagArmStart(PhantomData));
                let (a0,) = self.arms;
                $Fork {
                    chain: self.chain,
                    arms: (a0, arm),
                    _marker: PhantomData,
                }
            }
        }

        impl<$U, ForkOut, Chain, A0, C0, A1, C1>
            $Fork<$U, ForkOut, Chain, (TypedDagArm<ForkOut, A0, C0>, TypedDagArm<ForkOut, A1, C1>)>
        {
            /// Add a third arm to this fork.
            pub fn arm<AOut, ACh>(
                self,
                f: impl FnOnce(TypedDagArmStart<ForkOut>) -> TypedDagArm<ForkOut, AOut, ACh>,
            ) -> $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    TypedDagArm<ForkOut, A0, C0>,
                    TypedDagArm<ForkOut, A1, C1>,
                    TypedDagArm<ForkOut, AOut, ACh>,
                ),
            > {
                let arm = f(TypedDagArmStart(PhantomData));
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
                    TypedDagArm<ForkOut, A0, C0>,
                    TypedDagArm<ForkOut, A1, C1>,
                    TypedDagArm<ForkOut, A2, C2>,
                ),
            >
        {
            /// Add a fourth arm to this fork.
            pub fn arm<AOut, ACh>(
                self,
                f: impl FnOnce(TypedDagArmStart<ForkOut>) -> TypedDagArm<ForkOut, AOut, ACh>,
            ) -> $Fork<
                $U,
                ForkOut,
                Chain,
                (
                    TypedDagArm<ForkOut, A0, C0>,
                    TypedDagArm<ForkOut, A1, C1>,
                    TypedDagArm<ForkOut, A2, C2>,
                    TypedDagArm<ForkOut, AOut, ACh>,
                ),
            > {
                let arm = f(TypedDagArmStart(PhantomData));
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
            $Fork<$U, ForkOut, Chain, (TypedDagArm<ForkOut, A0, C0>, TypedDagArm<ForkOut, A1, C1>)>
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut) -> A0,
            C1: FnMut(&mut World, &ForkOut) -> A1,
        {
            /// Merge two arms with a merge stage.
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
                S: IntoMergeStage<(&'static A0, &'static A1), MOut, Params>,
                S::Stage: for<'x> MergeStageCall<(&'x A0, &'x A1), MOut>,
            {
                let mut chain = self.chain;
                let (a0, a1) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                let mut ms = f.into_merge_stage(registry);
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
            $Fork<$U, ForkOut, Chain, (TypedDagArm<ForkOut, (), C0>, TypedDagArm<ForkOut, (), C1>)>
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
                    TypedDagArm<ForkOut, A0, C0>,
                    TypedDagArm<ForkOut, A1, C1>,
                    TypedDagArm<ForkOut, A2, C2>,
                ),
            >
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut) -> A0,
            C1: FnMut(&mut World, &ForkOut) -> A1,
            C2: FnMut(&mut World, &ForkOut) -> A2,
        {
            /// Merge three arms with a merge stage.
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
                S: IntoMergeStage<(&'static A0, &'static A1, &'static A2), MOut, Params>,
                S::Stage: for<'x> MergeStageCall<(&'x A0, &'x A1, &'x A2), MOut>,
            {
                let mut chain = self.chain;
                let (a0, a1, a2) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                let mut c2 = a2.chain;
                let mut ms = f.into_merge_stage(registry);
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
                    TypedDagArm<ForkOut, (), C0>,
                    TypedDagArm<ForkOut, (), C1>,
                    TypedDagArm<ForkOut, (), C2>,
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
                    TypedDagArm<ForkOut, A0, C0>,
                    TypedDagArm<ForkOut, A1, C1>,
                    TypedDagArm<ForkOut, A2, C2>,
                    TypedDagArm<ForkOut, A3, C3>,
                ),
            >
        where
            Chain: FnMut(&mut World, $chain_input) -> ForkOut,
            C0: FnMut(&mut World, &ForkOut) -> A0,
            C1: FnMut(&mut World, &ForkOut) -> A1,
            C2: FnMut(&mut World, &ForkOut) -> A2,
            C3: FnMut(&mut World, &ForkOut) -> A3,
        {
            /// Merge four arms with a merge stage.
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
                S: IntoMergeStage<
                        (&'static A0, &'static A1, &'static A2, &'static A3),
                        MOut,
                        Params,
                    >,
                S::Stage: for<'x> MergeStageCall<(&'x A0, &'x A1, &'x A2, &'x A3), MOut>,
            {
                let mut chain = self.chain;
                let (a0, a1, a2, a3) = self.arms;
                let mut c0 = a0.chain;
                let mut c1 = a1.chain;
                let mut c2 = a2.chain;
                let mut c3 = a3.chain;
                let mut ms = f.into_merge_stage(registry);
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
                    TypedDagArm<ForkOut, (), C0>,
                    TypedDagArm<ForkOut, (), C1>,
                    TypedDagArm<ForkOut, (), C2>,
                    TypedDagArm<ForkOut, (), C3>,
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

impl_typed_dag_fork!(
    fork: TypedDagChainFork,
    output: TypedDagChain,
    upstream: E,
    chain_input: E,
    param: event: E
);

impl_typed_dag_fork!(
    fork: TypedDagArmFork,
    output: TypedDagArm,
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
    use crate::{Res, ResMut, Virtual, WorldBuilder};

    // -- Linear chain: A → B → C --

    #[test]
    fn dag_linear_chain() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64 * 2, reg);
        let mid = dag.stage(|val: &u64| *val + 1, reg);
        let sink = dag.stage(
            |mut out: ResMut<u64>, val: &u64| {
                *out = *val;
            },
            reg,
        );
        dag.edge(root, mid);
        dag.edge(mid, sink);

        let mut pipeline = dag.build();
        pipeline.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 11); // (5*2)+1
    }

    // -- Root only (single terminal stage) --

    #[test]
    fn dag_root_only() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        dag.root(
            |mut out: ResMut<u64>, x: u32| {
                *out = x as u64;
            },
            reg,
        );

        let mut pipeline = dag.build();
        pipeline.run(&mut world, 42u32);
        assert_eq!(*world.resource::<u64>(), 42);
    }

    // -- Fan-out: A → [B, C] --

    #[test]
    fn dag_fan_out() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<i64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64, reg);
        let a = dag.stage(
            |mut out: ResMut<u64>, val: &u64| {
                *out = *val * 2;
            },
            reg,
        );
        let b = dag.stage(
            |mut out: ResMut<i64>, val: &u64| {
                *out = *val as i64 * 3;
            },
            reg,
        );
        dag.edge(root, a);
        dag.edge(root, b);

        let mut pipeline = dag.build();
        pipeline.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 10);
        assert_eq!(*world.resource::<i64>(), 15);
    }

    // -- Diamond: A → [B, C] → D (fan-out + merge2) --

    #[test]
    fn dag_diamond() {
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64, reg);
        let double = dag.stage(|val: &u64| *val * 2, reg);
        let triple = dag.stage(|val: &u64| *val as i64 * 3, reg);
        dag.edge(root, double);
        dag.edge(root, triple);

        let _merge = dag.merge2(
            double,
            triple,
            |mut out: ResMut<String>, a: &u64, b: &i64| {
                *out = format!("{}+{}", a, b);
            },
            reg,
        );

        let mut pipeline = dag.build();
        pipeline.run(&mut world, 5u32);
        assert_eq!(world.resource::<String>().as_str(), "10+15");
    }

    // -- Implements Handler<E> --

    #[test]
    fn dag_implements_handler() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64, reg);
        let sink = dag.stage(
            |mut out: ResMut<u64>, val: &u64| {
                *out = *val;
            },
            reg,
        );
        dag.edge(root, sink);

        let mut pipeline = dag.build();

        // Handler trait dispatch
        Handler::run(&mut pipeline, &mut world, 99u32);
        assert_eq!(*world.resource::<u64>(), 99);
    }

    // -- Boxes into Virtual<E> --

    #[test]
    fn dag_boxable() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64, reg);
        let sink = dag.stage(
            |mut out: ResMut<u64>, val: &u64| {
                *out = *val;
            },
            reg,
        );
        dag.edge(root, sink);

        let mut boxed: Virtual<u32> = Box::new(dag.build());
        boxed.run(&mut world, 77u32);
        assert_eq!(*world.resource::<u64>(), 77);
    }

    // -- Cycle detection panics --

    #[test]
    #[should_panic(expected = "cycle detected")]
    fn dag_cycle_panics() {
        let wb = WorldBuilder::new();
        let world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64, reg);
        let a = dag.stage(|val: &u64| *val, reg);
        let b = dag.stage(|val: &u64| *val, reg);
        // root → a → b → a (cycle)
        dag.edge(root, a);
        dag.edge(a, b);
        dag.edge(b, a);

        // Terminal check would also fail, but cycle is detected first.
        dag.build();
    }

    // -- No root panics --

    #[test]
    #[should_panic(expected = "no root stage")]
    fn dag_no_root_panics() {
        let dag = DagBuilder::<u32>::new();
        dag.build();
    }

    // -- Double root panics --

    #[test]
    #[should_panic(expected = "root() called twice")]
    fn dag_double_root_panics() {
        let wb = WorldBuilder::new();
        let world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        dag.root(|x: u32| x as u64, reg);
        dag.root(|x: u32| x as i64, reg); // panics
    }

    // -- Terminal with non-unit output panics --

    #[test]
    #[should_panic(expected = "terminal stage")]
    fn dag_non_unit_terminal_panics() {
        let wb = WorldBuilder::new();
        let world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let _root = dag.root(|x: u32| x as u64, reg);
        // root outputs u64 but has no downstream → terminal with non-()
        dag.build();
    }

    // -- 3-way merge --

    #[test]
    fn dag_merge3() {
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64, reg);
        let a = dag.stage(|val: &u64| *val * 1, reg);
        let b = dag.stage(|val: &u64| *val * 2, reg);
        let c = dag.stage(|val: &u64| *val * 3, reg);
        dag.edge(root, a);
        dag.edge(root, b);
        dag.edge(root, c);

        let _merge = dag.merge3(
            a,
            b,
            c,
            |mut out: ResMut<String>, x: &u64, y: &u64, z: &u64| {
                *out = format!("{},{},{}", x, y, z);
            },
            reg,
        );

        let mut pipeline = dag.build();
        pipeline.run(&mut world, 10u32);
        assert_eq!(world.resource::<String>().as_str(), "10,20,30");
    }

    // -- Complex topology: fan-out + linear + merge --

    #[test]
    fn dag_complex_topology() {
        // root → [a, b]
        // a → c (linear)
        // b and c merge → sink
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64, reg);
        let a = dag.stage(|val: &u64| *val + 10, reg);
        let b = dag.stage(|val: &u64| *val + 20, reg);
        dag.edge(root, a);
        dag.edge(root, b);

        let c = dag.stage(|val: &u64| *val * 2, reg);
        dag.edge(a, c);

        let _merge = dag.merge2(
            b,
            c,
            |mut out: ResMut<u64>, bval: &u64, cval: &u64| {
                *out = *bval + *cval;
            },
            reg,
        );

        let mut pipeline = dag.build();
        pipeline.run(&mut world, 5u32);
        // a = 5+10 = 15, b = 5+20 = 25, c = 15*2 = 30
        // merge = 25 + 30 = 55
        assert_eq!(*world.resource::<u64>(), 55);
    }

    // -- Multiple dispatches reuse slots --

    #[test]
    fn dag_multiple_dispatches() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x as u64, reg);
        let sink = dag.stage(
            |mut out: ResMut<u64>, val: &u64| {
                *out = *val;
            },
            reg,
        );
        dag.edge(root, sink);

        let mut pipeline = dag.build();
        pipeline.run(&mut world, 1u32);
        assert_eq!(*world.resource::<u64>(), 1);
        pipeline.run(&mut world, 2u32);
        assert_eq!(*world.resource::<u64>(), 2);
        pipeline.run(&mut world, 3u32);
        assert_eq!(*world.resource::<u64>(), 3);
    }

    // -- Param access in stages --

    #[test]
    fn dag_param_access() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(10);
        wb.register::<String>(String::new());
        let mut world = wb.build();
        let reg = world.registry();

        fn scale(factor: Res<u64>, val: &u32) -> u64 {
            *factor * (*val as u64)
        }

        fn store(mut out: ResMut<String>, val: &u64) {
            *out = val.to_string();
        }

        let mut dag = DagBuilder::<u32>::new();
        let root = dag.root(|x: u32| x, reg);
        let scaled = dag.stage(scale, reg);
        let sink = dag.stage(store, reg);
        dag.edge(root, scaled);
        dag.edge(scaled, sink);

        let mut pipeline = dag.build();
        pipeline.run(&mut world, 7u32);
        assert_eq!(world.resource::<String>().as_str(), "70");
    }

    // -- Unreachable stage panics --

    #[test]
    #[should_panic(expected = "unreachable from root")]
    fn dag_unreachable_panics() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let world = wb.build();
        let reg = world.registry();

        let mut dag = DagBuilder::<u32>::new();
        dag.root(
            |mut out: ResMut<u64>, x: u32| {
                *out = x as u64;
            },
            reg,
        );
        // Disconnected stage — not connected to root via any edge.
        dag.stage(
            |mut out: ResMut<u64>, _val: &u64| {
                *out = 999;
            },
            reg,
        );

        dag.build();
    }

    // =========================================================================
    // Typed DAG tests
    // =========================================================================

    // -- Linear chains --

    #[test]
    fn typed_dag_linear_2() {
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

        let mut dag = TypedDagStart::<u32>::new()
            .root(root_mul2, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 10);
    }

    #[test]
    fn typed_dag_linear_3() {
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

        let mut dag = TypedDagStart::<u32>::new()
            .root(root_mul2, reg)
            .then(add_one, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 11); // (5*2)+1
    }

    #[test]
    fn typed_dag_linear_5() {
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

        let mut dag = TypedDagStart::<u32>::new()
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
    fn typed_dag_diamond() {
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

        let mut dag = TypedDagStart::<u32>::new()
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
    fn typed_dag_fan_out_join() {
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

        let mut dag = TypedDagStart::<u32>::new()
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
    fn typed_dag_nested_fork() {
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
        let mut dag = TypedDagStart::<u32>::new()
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
    fn typed_dag_complex_topology() {
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
        let mut dag = TypedDagStart::<u32>::new()
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
    fn typed_dag_boxable() {
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
            TypedDagStart::<u32>::new()
                .root(root_id, reg)
                .then(store, reg)
                .build(),
        );
        boxed.run(&mut world, 77u32);
        assert_eq!(*world.resource::<u64>(), 77);
    }

    // -- World access (Res<T>, ResMut<T>) in stages --

    #[test]
    fn typed_dag_world_access() {
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

        let mut dag = TypedDagStart::<u32>::new()
            .root(|x: u32| x, reg)
            .then(scale, reg)
            .then(store, reg)
            .build();

        dag.run(&mut world, 7u32);
        assert_eq!(world.resource::<String>().as_str(), "70");
    }

    // -- Root-only (terminal root outputting ()) --

    #[test]
    fn typed_dag_root_only() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();
        let reg = world.registry();

        let mut dag = TypedDagStart::<u32>::new()
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
    fn typed_dag_multiple_dispatches() {
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

        let mut dag = TypedDagStart::<u32>::new()
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
    fn typed_dag_3way_merge() {
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

        let mut dag = TypedDagStart::<u32>::new()
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
}
