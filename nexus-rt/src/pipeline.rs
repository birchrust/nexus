// Builder return types are necessarily complex — each combinator returns
// PipelineBuilder<In, Out, impl FnMut(...)>. Same pattern as iterator adapters.
#![allow(clippy::type_complexity)]

//! General transform chain with Option/Result convenience methods.
//!
//! [`PipelineStart`] begins a chain of stages that transform data using
//! closures. Each stage receives `&mut World` and the previous stage's
//! output. The core combinator is [`pipe`](PipelineBuilder::pipe), which
//! transforms the output type freely. When the output happens to be
//! `Option<T>` or `Result<T, E>`, additional convenience methods
//! (`map`, `and_then`, `filter`, `catch`, etc.) become available — matching
//! std's naming conventions.
//!
//! The builder is fully monomorphized: each combinator wraps the previous
//! chain in a new closure. No boxing until [`build()`](PipelineBuilder::build),
//! which erases the closure type to produce a [`Pipeline<In>`] that
//! implements [`System<In>`](crate::System).
//!
//! # Two dispatch modes
//!
//! - **`run()`** — call directly. No boxing, no `'static` on `In`.
//!   Works with borrowed inputs for zero-copy integration.
//! - **`build()`** — box into `Pipeline<In>`, implements `System<In>`.
//!   Requires `In: 'static`. Only available when the chain ends with `()`.

use std::marker::PhantomData;

use crate::system::System;
use crate::world::World;

// =============================================================================
// PipelineOutput — marker trait for build()
// =============================================================================

/// Marker trait restricting [`PipelineBuilder::build`] to pipelines that
/// produce `()`.
///
/// If your pipeline produces a value, add a final
/// `.pipe(|world, val| { /* write to World */ })` to consume the output.
#[diagnostic::on_unimplemented(
    message = "`build()` requires the pipeline output to be `()`",
    label = "this pipeline produces `{Self}`, not `()`",
    note = "add a final `.pipe(|world, val| {{ /* write to World */ }})` to consume the output"
)]
pub trait PipelineOutput {}
impl PipelineOutput for () {}

// =============================================================================
// PipelineStart — entry point
// =============================================================================

/// Entry point for building a typed pipeline.
///
/// `In` is the input type. Use [`.pipe()`](Self::pipe) for the first
/// transform — the return type determines where you end up (bare value,
/// `Option<T>`, `Result<T, E>`).
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, PipelineStart};
///
/// let mut world = WorldBuilder::new().build();
///
/// let mut pipeline = PipelineStart::<u32>::new()
///     .pipe(|_world, x| if x > 0 { Some(x * 2) } else { None })
///     .map(|_world, x| x + 1);
///
/// assert_eq!(pipeline.run(&mut world, 5), Some(11));
/// assert_eq!(pipeline.run(&mut world, 0), None);
/// ```
pub struct PipelineStart<In>(PhantomData<fn(In)>);

impl<In> PipelineStart<In> {
    /// Create a new pipeline entry point.
    pub fn new() -> Self {
        Self(PhantomData)
    }

    /// First transform. Return type determines where you end up.
    ///
    /// Return `Option<T>` to get Option combinators, `Result<T, E>` for
    /// Result combinators, or any other type for a bare value chain.
    pub fn pipe<Out>(
        self,
        f: impl FnMut(&mut World, In) -> Out + 'static,
    ) -> PipelineBuilder<In, Out, impl FnMut(&mut World, In) -> Out> {
        PipelineBuilder {
            chain: f,
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

/// Builder that composes pipeline stages via closure nesting.
///
/// `In` is the pipeline's input type (fixed). `Out` is the current output
/// type — can be anything. `F` is the concrete composed closure type
/// (opaque, never named by users).
///
/// Each combinator consumes `self`, captures the previous chain in a new
/// closure, and returns a new `PipelineBuilder`. The compiler monomorphizes
/// the entire chain — zero virtual dispatch through pipeline stages.
pub struct PipelineBuilder<In, Out, F> {
    chain: F,
    _marker: PhantomData<fn(In) -> Out>,
}

// =============================================================================
// Core — any Out
// =============================================================================

impl<In, Out, F> PipelineBuilder<In, Out, F>
where
    F: FnMut(&mut World, In) -> Out,
{
    /// Transform the output. Works with any output type.
    pub fn pipe<U>(
        self,
        mut f: impl FnMut(&mut World, Out) -> U + 'static,
    ) -> PipelineBuilder<In, U, impl FnMut(&mut World, In) -> U> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                let out = chain(world, input);
                f(world, out)
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
// build — when Out: PipelineOutput (i.e., Out = ())
// =============================================================================

impl<In: 'static, Out: PipelineOutput, F> PipelineBuilder<In, Out, F>
where
    F: FnMut(&mut World, In) -> Out + 'static,
{
    /// Box the composed closure and produce a [`Pipeline<In>`].
    ///
    /// Only available when the pipeline ends with `()`. If your chain
    /// produces a value, add a final `.pipe()` that writes it to World.
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
// Option helpers — PipelineBuilder<In, Option<T>, F>
// =============================================================================

impl<In, T, F> PipelineBuilder<In, Option<T>, F>
where
    F: FnMut(&mut World, In) -> Option<T>,
{
    /// Infallible transform on the inner value. std: `Option::map`
    pub fn map<U>(
        self,
        mut f: impl FnMut(&mut World, T) -> U + 'static,
    ) -> PipelineBuilder<In, Option<U>, impl FnMut(&mut World, In) -> Option<U>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| chain(world, input).map(|val| f(world, val)),
            _marker: PhantomData,
        }
    }

    /// Short-circuits on None. std: `Option::and_then`
    pub fn and_then<U>(
        self,
        mut f: impl FnMut(&mut World, T) -> Option<U> + 'static,
    ) -> PipelineBuilder<In, Option<U>, impl FnMut(&mut World, In) -> Option<U>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).and_then(|val| f(world, val))
            },
            _marker: PhantomData,
        }
    }

    /// Lazy fallback on None. std: `Option::or_else`
    pub fn or_else(
        self,
        mut f: impl FnMut(&mut World) -> Option<T> + 'static,
    ) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| chain(world, input).or_else(|| f(world)),
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

// -- flatten: Option<Option<U>> -> Option<U> ----------------------------------

impl<In, U, F> PipelineBuilder<In, Option<Option<U>>, F>
where
    F: FnMut(&mut World, In) -> Option<Option<U>>,
{
    /// Flatten nested Options. std: `Option::flatten`
    pub fn flatten(
        self,
    ) -> PipelineBuilder<In, Option<U>, impl FnMut(&mut World, In) -> Option<U>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| chain(world, input).flatten(),
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Result helpers — PipelineBuilder<In, Result<T, E>, F>
// =============================================================================

impl<In, T, E, F> PipelineBuilder<In, Result<T, E>, F>
where
    F: FnMut(&mut World, In) -> Result<T, E>,
{
    /// Infallible transform on Ok. std: `Result::map`
    pub fn map<U>(
        self,
        mut f: impl FnMut(&mut World, T) -> U + 'static,
    ) -> PipelineBuilder<In, Result<U, E>, impl FnMut(&mut World, In) -> Result<U, E>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| chain(world, input).map(|val| f(world, val)),
            _marker: PhantomData,
        }
    }

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

    /// Short-circuits on Err. std: `Result::and_then`
    pub fn and_then<U>(
        self,
        mut f: impl FnMut(&mut World, T) -> Result<U, E> + 'static,
    ) -> PipelineBuilder<In, Result<U, E>, impl FnMut(&mut World, In) -> Result<U, E>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| {
                chain(world, input).and_then(|val| f(world, val))
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

    /// Handle error and transition to Option.
    ///
    /// `Ok(val)` becomes `Some(val)` — handler not called.
    /// `Err(err)` calls the handler, then produces `None`.
    pub fn catch(
        self,
        mut f: impl FnMut(&mut World, E) + 'static,
    ) -> PipelineBuilder<In, Option<T>, impl FnMut(&mut World, In) -> Option<T>> {
        let mut chain = self.chain;
        PipelineBuilder {
            chain: move |world: &mut World, input: In| match chain(world, input) {
                Ok(val) => Some(val),
                Err(err) => {
                    f(world, err);
                    None
                }
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

// -- flatten: Result<Result<U, E>, E> -> Result<U, E> -------------------------

impl<In, U, E, F> PipelineBuilder<In, Result<Result<U, E>, E>, F>
where
    F: FnMut(&mut World, In) -> Result<Result<U, E>, E>,
{
    /// Flatten nested Results. std: `Result::flatten`
    pub fn flatten(
        self,
    ) -> PipelineBuilder<In, Result<U, E>, impl FnMut(&mut World, In) -> Result<U, E>> {
        let mut chain = self.chain;
        PipelineBuilder {
            // Result::flatten requires 1.89; MSRV is 1.85.
            chain: move |world: &mut World, input: In| chain(world, input).and_then(|inner| inner),
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Pipeline<In> — built pipeline
// =============================================================================

/// Built pipeline implementing [`System<In>`](crate::System).
///
/// Created by [`PipelineBuilder::build`]. The entire pipeline chain is
/// monomorphized at compile time. `build()` erases the closure type via
/// `Box<dyn FnMut>` for the `dyn System<In>` boundary.
///
/// One virtual dispatch per `run()` call; the pipeline body executes
/// with zero further indirection.
pub struct Pipeline<In> {
    chain: Box<dyn FnMut(&mut World, In)>,
}

impl<In: 'static> System<In> for Pipeline<In> {
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
    use crate::WorldBuilder;

    #[derive(Debug, Clone, PartialEq)]
    struct TestError(String);

    // =========================================================================
    // Core
    // =========================================================================

    #[test]
    fn pipe_transforms_value() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new().pipe(|_w, x| x * 2);
        assert_eq!(p.run(&mut world, 5), 10);
    }

    #[test]
    fn pipe_chains_transforms() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| x * 2)
            .pipe(|_w, x| x + 1);
        assert_eq!(p.run(&mut world, 5), 11);
    }

    #[test]
    fn pipe_to_unit_and_build() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        let mut pipeline = PipelineStart::<u32>::new()
            .pipe(|w, x| {
                *w.resource_mut::<u64>() += u64::from(x);
            })
            .build();

        pipeline.run(&mut world, 10);
        assert_eq!(*world.resource::<u64>(), 10);
    }

    #[test]
    fn run_returns_output() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new().pipe(|_w, x| x * 3);
        let result: u32 = p.run(&mut world, 7);
        assert_eq!(result, 21);
    }

    #[test]
    fn build_produces_system() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        let mut pipeline: Pipeline<u32> = PipelineStart::<u32>::new()
            .pipe(|w, x| {
                *w.resource_mut::<u64>() += u64::from(x);
            })
            .build();

        pipeline.run(&mut world, 10);
        pipeline.run(&mut world, 5);
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn pipeline_in_world_via_with_mut() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);

        let pipeline = PipelineStart::<u32>::new()
            .pipe(|w, x| {
                *w.resource_mut::<u64>() += u64::from(x);
            })
            .build();

        wb.register::<Pipeline<u32>>(pipeline);
        let mut world = wb.build();

        world.with_mut::<Pipeline<u32>, _>(|pipeline, world| {
            pipeline.run(world, 7);
        });
        assert_eq!(*world.resource::<u64>(), 7);
    }

    // =========================================================================
    // Option
    // =========================================================================

    #[test]
    fn map_transforms_inner() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x))
            .map(|_w, x| x * 2);
        assert_eq!(p.run(&mut world, 5), Some(10));
    }

    #[test]
    fn map_skipped_on_none() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .map(|w, x| {
                *w.resource_mut::<bool>() = true;
                x
            });
        assert_eq!(p.run(&mut world, 5), None);
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn and_then_chains() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x * 2))
            .and_then(|_w, x| Some(x + 1));
        assert_eq!(p.run(&mut world, 5), Some(11));
    }

    #[test]
    fn and_then_short_circuits_on_none() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .and_then(|w, x| {
                *w.resource_mut::<bool>() = true;
                Some(x)
            });
        p.run(&mut world, 5);
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn or_else_provides_fallback() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .or_else(|_w| Some(42));
        assert_eq!(p.run(&mut world, 0), Some(42));
    }

    #[test]
    fn or_else_skipped_on_some() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x))
            .or_else(|w| {
                *w.resource_mut::<bool>() = true;
                Some(99)
            });
        assert_eq!(p.run(&mut world, 5), Some(5));
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn filter_keeps_matching() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x))
            .filter(|_w, x| *x > 3);
        assert_eq!(p.run(&mut world, 5), Some(5));
    }

    #[test]
    fn filter_drops_non_matching() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x))
            .filter(|_w, x| *x > 10);
        assert_eq!(p.run(&mut world, 5), None);
    }

    #[test]
    fn inspect_side_effect() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x))
            .inspect(|w, x| {
                *w.resource_mut::<u64>() = u64::from(*x);
            });
        assert_eq!(p.run(&mut world, 7), Some(7));
        assert_eq!(*world.resource::<u64>(), 7);
    }

    #[test]
    fn inspect_skipped_on_none() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .inspect(|w, _x| {
                *w.resource_mut::<bool>() = true;
            });
        assert_eq!(p.run(&mut world, 0), None);
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn on_none_side_effect() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .on_none(|w| {
                *w.resource_mut::<bool>() = true;
            });
        assert_eq!(p.run(&mut world, 0), None);
        assert!(*world.resource::<bool>());
    }

    #[test]
    fn on_none_skipped_on_some() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x))
            .on_none(|w| {
                *w.resource_mut::<bool>() = true;
            });
        assert_eq!(p.run(&mut world, 5), Some(5));
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn ok_or_converts_none_to_err() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .ok_or(TestError("missing".into()));
        assert_eq!(p.run(&mut world, 0), Err(TestError("missing".into())));
    }

    #[test]
    fn ok_or_passes_some_as_ok() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x))
            .ok_or(TestError("missing".into()));
        assert_eq!(p.run(&mut world, 5), Ok(5));
    }

    #[test]
    fn ok_or_else_lazy_error() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(42);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .ok_or_else(|w| *w.resource::<u64>());
        assert_eq!(p.run(&mut world, 0), Err(42));
    }

    #[test]
    fn unwrap_or_exits_option() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 0), 99);
    }

    #[test]
    fn unwrap_or_passes_some() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(x))
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 5), 5);
    }

    #[test]
    fn unwrap_or_else_exits_option() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(42);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<u32> { None })
            .unwrap_or_else(|w| *w.resource::<u64>() as u32);
        assert_eq!(p.run(&mut world, 0), 42);
    }

    #[test]
    fn flatten_nested_option() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| Some(Some(x * 2)))
            .flatten();
        assert_eq!(p.run(&mut world, 3), Some(6));

        // Inner None
        let mut p2 = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Option<Option<u32>> { Some(None) })
            .flatten();
        assert_eq!(p2.run(&mut world, 3), None);
    }

    // =========================================================================
    // Result
    // =========================================================================

    #[test]
    fn map_transforms_ok() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| -> Result<u32, TestError> { Ok(x) })
            .map(|_w, x| x * 3);
        assert_eq!(p.run(&mut world, 4), Ok(12));
    }

    #[test]
    fn map_skipped_on_err() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, TestError> { Err(TestError("fail".into())) })
            .map(|w, x| {
                *w.resource_mut::<bool>() = true;
                x
            });
        assert!(p.run(&mut world, 5).is_err());
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn map_err_transforms_error() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, String> { Err("oops".into()) })
            .map_err(|_w, e| e.len());
        assert_eq!(p.run(&mut world, 0), Err(4));
    }

    #[test]
    fn and_then_chains_results() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| -> Result<u32, TestError> { Ok(x * 2) })
            .and_then(|_w, x| -> Result<u32, TestError> { Ok(x + 1) });
        assert_eq!(p.run(&mut world, 5), Ok(11));
    }

    #[test]
    fn and_then_short_circuits_on_err() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, TestError> { Err(TestError("early".into())) })
            .and_then(|w, x| -> Result<u32, TestError> {
                *w.resource_mut::<bool>() = true;
                Ok(x)
            });
        assert!(p.run(&mut world, 5).is_err());
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn or_else_recovers_from_err() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, TestError> { Err(TestError("fail".into())) })
            .or_else(|_w, _err| -> Result<u32, TestError> { Ok(42) });
        assert_eq!(p.run(&mut world, 0), Ok(42));
    }

    #[test]
    fn or_else_skipped_on_ok() {
        let mut wb = WorldBuilder::new();
        wb.register::<bool>(false);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| -> Result<u32, TestError> { Ok(x) })
            .or_else(|w, _err| -> Result<u32, TestError> {
                *w.resource_mut::<bool>() = true;
                Ok(99)
            });
        assert_eq!(p.run(&mut world, 5), Ok(5));
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn inspect_on_ok() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| -> Result<u32, TestError> { Ok(x) })
            .inspect(|w, x| {
                *w.resource_mut::<u64>() = u64::from(*x);
            });
        assert_eq!(p.run(&mut world, 7), Ok(7));
        assert_eq!(*world.resource::<u64>(), 7);
    }

    #[test]
    fn inspect_err_on_err() {
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, String> { Err("bad".into()) })
            .inspect_err(|w, e| {
                *w.resource_mut::<String>() = e.clone();
            });
        assert!(p.run(&mut world, 0).is_err());
        assert_eq!(world.resource::<String>().as_str(), "bad");
    }

    #[test]
    fn catch_handles_error() {
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, String> { Err("caught".into()) })
            .catch(|w, err| {
                *w.resource_mut::<String>() = err;
            });
        assert_eq!(p.run(&mut world, 0), None);
        assert_eq!(world.resource::<String>().as_str(), "caught");
    }

    #[test]
    fn catch_passes_ok_as_some() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| -> Result<u32, String> { Ok(x) })
            .catch(|_w, _err| {});
        assert_eq!(p.run(&mut world, 5), Some(5));
    }

    #[test]
    fn ok_discards_error() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, String> { Err("gone".into()) })
            .ok();
        assert_eq!(p.run(&mut world, 0), None);
    }

    #[test]
    fn ok_preserves_value() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| -> Result<u32, String> { Ok(x) })
            .ok();
        assert_eq!(p.run(&mut world, 5), Some(5));
    }

    #[test]
    fn unwrap_or_exits_result() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, String> { Err("fail".into()) })
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 0), 99);
    }

    #[test]
    fn unwrap_or_passes_ok() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| -> Result<u32, String> { Ok(x) })
            .unwrap_or(99);
        assert_eq!(p.run(&mut world, 5), 5);
    }

    #[test]
    fn unwrap_or_else_exits_result() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, String> { Err("fail".into()) })
            .unwrap_or_else(|_w, err| err.len() as u32);
        assert_eq!(p.run(&mut world, 0), 4);
    }

    #[test]
    fn flatten_nested_result() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| -> Result<Result<u32, TestError>, TestError> { Ok(Ok(x * 2)) })
            .flatten();
        assert_eq!(p.run(&mut world, 3), Ok(6));

        // Inner Err
        let mut p2 = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<Result<u32, TestError>, TestError> {
                Ok(Err(TestError("inner".into())))
            })
            .flatten();
        assert_eq!(p2.run(&mut world, 3), Err(TestError("inner".into())));
    }

    // =========================================================================
    // Transitions
    // =========================================================================

    #[test]
    fn pipe_to_option_to_result() {
        let mut world = WorldBuilder::new().build();
        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, x| x * 2) // bare
            .pipe(|_w, x| Some(x)) // -> Option
            .ok_or("err") // -> Result
            .ok(); // -> Option
        assert_eq!(p.run(&mut world, 5), Some(10));
    }

    #[test]
    fn catch_to_option_and_continue() {
        let mut wb = WorldBuilder::new();
        wb.register::<String>(String::new());
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new()
            .pipe(|_w, _x| -> Result<u32, String> { Err("oops".into()) })
            .catch(|w, err| {
                *w.resource_mut::<String>() = err;
            })
            .map(|_w, x| x * 2);
        assert_eq!(p.run(&mut world, 5), None);
        assert_eq!(world.resource::<String>().as_str(), "oops");
    }

    // =========================================================================
    // Direct run
    // =========================================================================

    #[test]
    fn run_with_borrowed_input() {
        let mut world = WorldBuilder::new().build();
        // data declared before p so it outlives the pipeline (drop order).
        let data = vec![1u8, 2, 3, 4, 5];
        let mut p = PipelineStart::<&[u8]>::new().pipe(|_w, bytes: &[u8]| {
            if bytes.len() >= 4 {
                Some(bytes.len())
            } else {
                None
            }
        });
        assert_eq!(p.run(&mut world, &data), Some(5));
        assert_eq!(p.run(&mut world, &data[..2]), None);
    }

    #[test]
    fn run_zero_copy_intermediate() {
        let mut world = WorldBuilder::new().build();
        // data declared before p so it outlives the pipeline (drop order).
        let data = b"hello";
        let bad = vec![0xFF, 0xFE];
        let mut p = PipelineStart::<&[u8]>::new()
            .pipe(|_w, bytes: &[u8]| std::str::from_utf8(bytes).ok())
            .map(|_w, s: &str| s.len());

        assert_eq!(p.run(&mut world, &data[..]), Some(5));
        assert_eq!(p.run(&mut world, &bad), None);
    }

    // =========================================================================
    // World access
    // =========================================================================

    #[test]
    fn stage_reads_resource() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(42);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new().pipe(|w, _x| *w.resource::<u64>());
        assert_eq!(p.run(&mut world, 0), 42);
    }

    #[test]
    fn stage_writes_resource() {
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        let mut world = wb.build();

        let mut p = PipelineStart::<u32>::new().pipe(|w, x| {
            *w.resource_mut::<u64>() += u64::from(x);
        });
        p.run(&mut world, 10);
        p.run(&mut world, 5);
        assert_eq!(*world.resource::<u64>(), 15);
    }
}
