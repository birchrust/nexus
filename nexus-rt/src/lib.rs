//! # nexus-rt
//!
//! Runtime primitives for single-threaded, event-driven systems.
//!
//! `nexus-rt` provides the building blocks for constructing runtimes where
//! user code runs as handlers dispatched over shared state. It is **not** an
//! async runtime — there is no task scheduler, no work stealing, no `Future`
//! polling. Instead, it provides:
//!
//! - **World** — [`World`] is a unified type-erased singleton store. Each
//!   registered type gets a dense index ([`ResourceId`]) for ~3-cycle
//!   dispatch-time access.
//!
//! - **Resources** — [`Res`] and [`ResMut`] are what users see in handler
//!   function signatures. They deref to the inner value transparently.
//!
//! - **Handlers** — The [`SystemParam`] trait resolves state at build time
//!   and produces references at dispatch time. [`IntoHandler`] converts
//!   plain functions into [`Handler`] trait objects for type-erased dispatch.
//!
//! - **Pipeline** — [`PipelineStart`] begins a typed per-event composition
//!   chain. Stages transform data using `Option` and `Result` for flow
//!   control. [`Pipeline`] implements [`Handler`] for direct or boxed dispatch.
//!   [`BatchPipeline`] owns a pre-allocated input buffer and runs each item
//!   through the same chain independently — errors on one item don't affect
//!   subsequent items.
//!
//! - **Driver** — [`Driver`] is the install-time trait for event sources.
//!   `install()` registers resources into [`WorldBuilder`] and returns a
//!   concrete handle whose `poll()` method drives the event lifecycle.
//!
//! - **Plugin** — [`Plugin`] is a composable unit of resource registration.
//!   [`WorldBuilder::install_plugin`] consumes a plugin to configure state.
//!
//! # Quick Start
//!
//! ```
//! use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler};
//!
//! let mut builder = WorldBuilder::new();
//! builder.register::<u64>(0);
//! let mut world = builder.build();
//!
//! fn tick(mut counter: ResMut<u64>, event: u32) {
//!     *counter += event as u64;
//! }
//!
//! let mut handler = tick.into_handler(world.registry());
//!
//! handler.run(&mut world, 10u32);
//!
//! assert_eq!(*world.resource::<u64>(), 10);
//! ```
//!
//! # Safety
//!
//! The low-level `get` / `get_mut` methods on [`World`] are `unsafe` and
//! intended for framework internals. The caller must ensure:
//!
//! 1. The ID was obtained from the same builder that produced the container.
//! 2. The type parameter matches the type registered at that ID.
//! 3. No mutable aliasing — at most one `&mut T` exists per value at any time.
//!
//! User-facing APIs (`resource`, `resource_mut`, `Handler::run`) are fully safe.

#![warn(missing_docs)]

mod callback;
mod driver;
pub mod pipeline;
mod plugin;
mod resource;
mod system;
#[cfg(feature = "timer")]
pub mod timer;
mod world;

pub use callback::{Callback, IntoCallback};
pub use driver::Driver;
pub use pipeline::{
    BatchPipeline, IntoStage, Pipeline, PipelineBuilder, PipelineOutput, PipelineStart,
};
pub use plugin::Plugin;
pub use resource::{Res, ResMut};
pub use system::{CtxFree, Handler, HandlerFn, IntoHandler, Local, RegistryRef, SystemParam};
pub use world::{Registry, ResourceId, Sequence, World, WorldBuilder};

/// Type alias for a boxed, type-erased [`Handler`].
pub type Virtual<E> = Box<dyn Handler<E>>;

/// Type alias for an inline [`Handler`] using [`nexus_smartptr::Flat`].
///
/// Stores the handler inline (no heap allocation). Panics if the concrete
/// handler doesn't fit in the buffer.
#[cfg(feature = "smartptr")]
pub type FlatVirtual<E, B = nexus_smartptr::B64> = nexus_smartptr::Flat<dyn Handler<E>, B>;

/// Type alias for an inline [`Handler`] with heap fallback using [`nexus_smartptr::Flex`].
///
/// Stores inline if the handler fits, otherwise heap-allocates.
#[cfg(feature = "smartptr")]
pub type FlexVirtual<E, B = nexus_smartptr::B64> = nexus_smartptr::Flex<dyn Handler<E>, B>;

#[cfg(feature = "timer")]
pub use timer::{BoxedTimers, Periodic, TimerConfig, TimerDriver, TimerHandle, TimerWheel};

#[cfg(all(feature = "timer", feature = "smartptr"))]
pub use timer::{FlexTimerWheel, FlexTimers, InlineTimerWheel, InlineTimers};
