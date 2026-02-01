//! Internal implementation details.
//!
//! This module contains traits and utilities used internally by the
//! collection implementations. The traits are public (for use in generic
//! bounds) but sealed - users cannot implement them.

// TODO(phase-2): Remove these allows once ListStorage uses SlabOps
#![allow(dead_code)]
#![allow(unused_imports)]

pub(crate) mod slab_ops;

// Re-export publicly for use in generic bounds (but traits are sealed)
pub use slab_ops::{SlabOps, SlotOps};
