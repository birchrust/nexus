//! # nexus-rt
//!
//! Runtime primitives for single-threaded, event-driven systems.
//!
//! `nexus-rt` provides the building blocks for constructing runtimes where
//! user code runs as systems dispatched over shared state. It is **not** an
//! async runtime — there is no task scheduler, no work stealing, no `Future`
//! polling. Instead, it provides:
//!
//! - **World** — [`World`] is a unified type-erased singleton store. Each
//!   registered type gets a dense index ([`ResourceId`]) for ~3-cycle
//!   dispatch-time access.
//!
//! - **Resources** — [`Res`] and [`ResMut`] are what users see in system
//!   function signatures. They deref to the inner value transparently.
//!
//! - **Systems** — The [`SystemParam`] trait resolves state at build time
//!   and produces references at dispatch time. [`IntoSystem`] converts
//!   plain functions into [`System`] trait objects for type-erased dispatch.
//!
//! - **Pipeline** — [`PipelineStart`] begins a typed per-event composition
//!   chain. Stages transform data using `Option` and `Result` for flow
//!   control. [`Pipeline`] implements [`System`] for direct or boxed dispatch.
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
//! use nexus_rt::{WorldBuilder, ResMut, IntoSystem, System};
//!
//! let mut builder = WorldBuilder::new();
//! builder.register::<u64>(0);
//! let mut world = builder.build();
//!
//! fn tick(mut counter: ResMut<u64>, event: u32) {
//!     *counter += event as u64;
//! }
//!
//! let mut system = tick.into_system(world.registry_mut());
//!
//! system.run(&mut world, 10u32);
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
//! User-facing APIs (`resource`, `resource_mut`, `System::run`) are fully safe.

#![warn(missing_docs)]

mod callback;
mod driver;
pub mod pipeline;
mod plugin;
mod resource;
mod system;
mod world;

pub use callback::{Callback, IntoCallback};
pub use driver::Driver;
pub use pipeline::{
    BatchPipeline, IntoStage, Pipeline, PipelineBuilder, PipelineOutput, PipelineStart,
};
pub use plugin::Plugin;
pub use resource::{Res, ResMut};
pub use system::{FunctionSystem, IntoSystem, Local, System, SystemParam};
pub use world::{Registry, ResourceId, Sequence, World, WorldBuilder};
