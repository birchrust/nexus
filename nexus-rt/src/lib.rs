//! # nexus-rt
//!
//! Runtime primitives for single-threaded, event-driven systems.
//!
//! `nexus-rt` provides the building blocks for constructing runtimes where
//! user code runs as systems dispatched over shared state. It is **not** an
//! async runtime — there is no task scheduler, no work stealing, no `Future`
//! polling. Instead, it provides:
//!
//! - **Stores** — [`Components`] and [`Drivers`] are type-erased singleton
//!   stores. Each registered type gets a dense index ([`ComponentId`] /
//!   [`DriverId`]) for ~3-cycle dispatch-time access.
//!
//! - **Fetch** — The [`Fetch`] trait resolves state at build time and
//!   produces references at dispatch time. Eliminates manual resolve+cast
//!   boilerplate.
//!
//! - **Parameters** — [`Comp`] and [`Ctx`] are what users see in system
//!   function signatures. They deref to the inner value transparently.
//!
//! # Quick Start
//!
//! ```
//! use nexus_rt::{Components, ComponentsBuilder, Drivers, DriversBuilder};
//!
//! let components = Components::builder()
//!     .register::<u64>(0)
//!     .register::<bool>(true)
//!     .build();
//!
//! let drivers = Drivers::builder()
//!     .register::<String>(String::from("timer"))
//!     .build();
//!
//! // Hot path: ~3 cycles per fetch via dense index
//! let counter_id = components.id::<u64>();
//! let driver_id = drivers.id::<String>();
//!
//! unsafe {
//!     *components.get_mut::<u64>(counter_id) += 1;
//!     assert_eq!(*components.get::<u64>(counter_id), 1);
//!     assert_eq!(drivers.get::<String>(driver_id), "timer");
//! }
//! ```
//!
//! # Fetch
//!
//! ```
//! use nexus_rt::{Components, ComponentsBuilder, Fetch};
//!
//! let components = ComponentsBuilder::new()
//!     .register::<u64>(42)
//!     .build();
//!
//! // Build time: resolve state once
//! let state = <&u64 as Fetch<Components>>::init(&components);
//!
//! // Dispatch time: fetch reference (~3 cycles)
//! let val = unsafe { <&u64 as Fetch<Components>>::fetch(&components, &state) };
//! assert_eq!(*val, 42);
//! ```
//!
//! # Safety
//!
//! The `get` / `get_mut` methods on both stores are `unsafe`. The caller
//! must ensure:
//!
//! 1. The ID was obtained from the same builder that produced the container.
//! 2. The type parameter matches the type registered at that ID.
//! 3. No mutable aliasing — at most one `&mut T` exists per value at any time.
//!
//! These invariants are naturally upheld by single-threaded sequential dispatch.

#![warn(missing_docs)]

mod components;
mod drivers;
mod fetch;

pub use components::{ComponentId, Components, ComponentsBuilder};
pub use drivers::{DriverId, Drivers, DriversBuilder};
pub use fetch::{Comp, Ctx, Fetch};
