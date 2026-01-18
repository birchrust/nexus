//! # nexus-pool
//!
//! Object pools with LIFO reuse for cache-hot recycling.
//!
//! # Pool Types
//!
//! ## Single-threaded (`local`)
//!
//! | Type | Capacity |
//! |------|----------|
//! | [`local::BoundedPool`] | Fixed |
//! | [`local::Pool`] | Growable |
//!
//! ## Thread-safe (`sync`) — coming soon
//!
//! | Type | Capacity |
//! |------|----------|
//! | `sync::BoundedPool` | Fixed |
//! | `sync::Pool` | Growable |

pub mod local;
