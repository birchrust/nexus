//! Internal implementation details.
//!
//! This module contains traits and utilities used internally by the
//! collection implementations.

pub(crate) mod list_storage;

// Re-export the storage trait for use in generic bounds
pub use list_storage::ListStorage;
