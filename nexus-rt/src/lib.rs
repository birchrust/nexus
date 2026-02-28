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
//! - **Events** — [`Events`], [`EventWriter`], and [`EventReader`] provide
//!   simple event buffer types that integrate as system parameters.
//!
//! - **Scheduler** — [`Scheduler`] toposorts systems by ordering constraints
//!   and dispatches them with automatic skip propagation via tick-based
//!   change detection.
//!
//! - **Pipeline** — [`PipelineStart`] begins a typed per-event composition
//!   chain. Stages transform data using `Option` and `Result` for flow
//!   control. [`Pipeline`] implements [`System`] for Scheduler integration.
//!
//! - **Plugin / App** — [`Plugin`] is a composable unit of registration.
//!   [`App`] ties [`WorldBuilder`] and [`SchedulerBuilder`] together
//!   for ergonomic setup.
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
//! let mut system = tick.into_system(world.registry());
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
//! User-facing APIs (`resource`, `resource_mut`, `with_mut`, `System::run`)
//! are fully safe.

#![warn(missing_docs)]

mod app;
mod event;
pub mod pipeline;
mod plugin;
mod resource;
mod scheduler;
mod system;
mod world;

pub use app::App;
pub use event::{EventReader, EventWriter, Events};
pub use pipeline::{Pipeline, PipelineBuilder, PipelineOutput, PipelineStart};
pub use plugin::Plugin;
pub use resource::{Res, ResMut};
pub use scheduler::{Scheduler, SchedulerBuilder, SystemId};
pub use system::{FunctionSystem, IntoSystem, Local, System, SystemParam};
pub use world::{Registry, ResourceId, Tick, World, WorldBuilder};
