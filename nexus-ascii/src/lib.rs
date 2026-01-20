//! Fixed-capacity ASCII strings for high-performance systems.
//!
//! This crate provides stack-allocated, fixed-capacity ASCII string types
//! optimized for trading systems and other latency-sensitive applications.

mod str_ref;
mod string;

// Hash module is public for benchmarking in examples
pub mod hash;
