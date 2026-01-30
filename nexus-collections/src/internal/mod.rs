//! Internal implementation details.
//!
//! This module contains traits and utilities used internally by the
//! collection implementations. Nothing here is part of the public API.

// TODO(phase-2): Remove these allows once ListStorage uses SlabOps
#![allow(dead_code)]
#![allow(unused_imports)]

pub(crate) mod slab_ops;

pub(crate) use slab_ops::SlabOps;
