//! Backing integer trait for `Decimal<B, D>`.
//!
//! Sealed — only `i32`, `i64`, and `i128` implement this. Exists for
//! the struct bound (`Decimal<B: Backing, D>`), not for method dispatch.
//! The `impl_decimal_*!` macros handle all method generation.

use core::hash::Hash;

mod sealed {
    pub trait Sealed {}
    impl Sealed for i32 {}
    impl Sealed for i64 {}
    impl Sealed for i128 {}
}

/// Marker trait for valid decimal backing types.
///
/// Only `i32`, `i64`, and `i128` implement this trait. It is sealed
/// and cannot be implemented for external types.
pub trait Backing: Copy + Eq + Ord + Hash + Default + sealed::Sealed {}

impl Backing for i32 {}
impl Backing for i64 {}
impl Backing for i128 {}
