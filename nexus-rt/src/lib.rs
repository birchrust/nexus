//! # nexus-rt
//!
//! Runtime primitives for single-threaded, event-driven systems.
//!
//! `nexus-rt` provides the building blocks for constructing runtimes where
//! user code runs as systems dispatched over shared component state. It is
//! **not** an async runtime — there is no task scheduler, no work stealing,
//! no `Future` polling. Instead, it provides:
//!
//! - **[`Components`]** — A type-erased singleton store where each component
//!   type gets a dense index ([`ComponentId`]) for ~3-cycle dispatch.
//!
//! # Quick Start
//!
//! ```
//! use nexus_rt::{Components, ComponentsBuilder};
//!
//! let components = Components::builder()
//!     .register::<u64>(0)
//!     .register::<bool>(true)
//!     .build();
//!
//! let counter_id = components.id::<u64>();
//! let flag_id = components.id::<bool>();
//!
//! // Hot path: ~3 cycles per fetch via dense index
//! unsafe {
//!     *components.get_mut::<u64>(counter_id) += 1;
//!     assert_eq!(*components.get::<u64>(counter_id), 1);
//!     assert_eq!(*components.get::<bool>(flag_id), true);
//! }
//! ```
//!
//! # Performance
//!
//! | Strategy | p50 (cycles) | p99 | p999 |
//! |----------|-------------|-----|------|
//! | Direct struct field (compile-time) | 2 | 5 | 9 |
//! | **Dense index + `get_unchecked`** (this design) | **3** | **5** | **7** |
//! | Dense index + bounds check | 3 | 8 | 17 |
//! | `Box<dyn Any>` + downcast | 8 | 13 | 24 |
//!
//! # Safety
//!
//! The `get` / `get_mut` methods on [`Components`] are `unsafe`. The caller
//! must ensure:
//!
//! 1. The [`ComponentId`] was obtained from the same builder that produced
//!    the container.
//! 2. The type parameter matches the type registered at that ID.
//! 3. No mutable aliasing — at most one `&mut T` exists per component at
//!    any time.
//!
//! These invariants are naturally upheld by single-threaded sequential dispatch.

#![warn(missing_docs)]

mod components;

pub use components::{ComponentId, Components, ComponentsBuilder};
