//! Inline smart pointers for `?Sized` types.
//!
//! `nexus-smartptr` provides [`Flat`] and [`Flex`] — smart pointers that
//! store trait objects (or other `?Sized` types) inline, avoiding heap
//! allocation for small values.
//!
//! - [`Flat<T, B>`] — inline only. Panics if the concrete type doesn't fit.
//! - [`Flex<T, B>`] — inline with heap fallback. Never panics.
//!
//! `B` is a buffer marker type (`B32`, `B64`, etc.) whose name is the
//! total size in bytes. `size_of::<Flat<dyn Trait, B32>>() == 32`.
//!
//! # Construction
//!
//! Use the [`flat!`] and [`flex!`] macros for `?Sized` types:
//!
//! ```
//! use nexus_smartptr::{flat, flex, Flat, Flex, B32};
//!
//! trait Greet {
//!     fn greet(&self) -> &str;
//! }
//!
//! struct Hello;
//! impl Greet for Hello {
//!     fn greet(&self) -> &str { "hello" }
//! }
//!
//! // Inline only — panics if Hello doesn't fit
//! let f: Flat<dyn Greet, B32> = flat!(Hello);
//! assert_eq!(f.greet(), "hello");
//!
//! // Inline with heap fallback
//! let f: Flex<dyn Greet, B32> = flex!(Hello);
//! assert!(f.is_inline());
//! assert_eq!(f.greet(), "hello");
//! ```
//!
//! For `Sized` types, use [`Flat::new`] or [`Flex::new`] directly:
//!
//! ```
//! use nexus_smartptr::{Flat, Flex, B32};
//!
//! let f: Flat<u64, B32> = Flat::new(42);
//! assert_eq!(*f, 42);
//!
//! let f: Flex<u64, B32> = Flex::new(42);
//! assert_eq!(*f, 42);
//! ```
//!
//! # Sizing Reference
//!
//! | Marker | Total size | `?Sized` value capacity | `Sized` capacity |
//! |--------|-----------|------------------------|-----------------|
//! | `B16`  | 16 bytes  | 8 bytes   | 16 bytes |
//! | `B32`  | 32 bytes  | 24 bytes  | 32 bytes |
//! | `B64`  | 64 bytes  | 56 bytes  | 64 bytes |
//! | `B128` | 128 bytes | 120 bytes | 128 bytes |
//! | `B256` | 256 bytes | 248 bytes | 256 bytes |
//!
//! For `Flex`, subtract an additional 8 bytes (heap pointer / discriminant).

#![warn(missing_docs)]

mod flat;
mod flex;
mod meta;

pub use flat::Flat;
pub use flex::Flex;

/// Marker trait for inline buffer sizes.
///
/// Each implementor is a zero-sized type representing a fixed buffer capacity
/// with `usize` alignment (8 bytes on 64-bit).
///
/// Use [`define_buffer!`] to create custom sizes.
///
/// # Safety
///
/// `CAPACITY` must equal `size_of::<Self>()`. Implementors must be
/// `repr(C, align(8))` with the correct size.
pub unsafe trait Buffer: Sized {
    /// Total buffer capacity in bytes.
    const CAPACITY: usize;
}

/// Defines a buffer marker type with a given byte capacity.
///
/// The size must be a multiple of 8 (the alignment requirement).
///
/// # Examples
///
/// ```
/// nexus_smartptr::define_buffer!(B512, 512);
/// ```
///
/// Non-multiples of 8 are rejected at compile time:
///
/// ```compile_fail
/// nexus_smartptr::define_buffer!(B12, 12);
/// ```
#[macro_export]
macro_rules! define_buffer {
    ($name:ident, $bytes:literal) => {
        /// Buffer marker:
        #[doc = concat!(stringify!($bytes), " bytes, align(8).")]
        #[repr(C, align(8))]
        pub struct $name([u8; $bytes]);

        // Validate that declared size matches actual struct size.
        // Fails if $bytes is not a multiple of 8 (alignment padding changes size).
        const _: () = assert!(
            core::mem::size_of::<$name>() == $bytes,
            "buffer size must be a multiple of 8 (alignment requirement)"
        );

        // SAFETY: CAPACITY == size_of::<Self>() == $bytes, verified above.
        // repr(C, align(8)) guarantees layout.
        unsafe impl $crate::Buffer for $name {
            const CAPACITY: usize = $bytes;
        }
    };
}

define_buffer!(B16, 16);
define_buffer!(B32, 32);
define_buffer!(B48, 48);
define_buffer!(B64, 64);
define_buffer!(B128, 128);
define_buffer!(B256, 256);
define_buffer!(B512, 512);
define_buffer!(B1024, 1024);
define_buffer!(B2048, 2048);

// -- Type aliases for convenience --

/// `Flat` with 16-byte buffer.
pub type Flat16<T> = Flat<T, B16>;
/// `Flat` with 32-byte buffer.
pub type Flat32<T> = Flat<T, B32>;
/// `Flat` with 48-byte buffer.
pub type Flat48<T> = Flat<T, B48>;
/// `Flat` with 64-byte buffer.
pub type Flat64<T> = Flat<T, B64>;
/// `Flat` with 128-byte buffer.
pub type Flat128<T> = Flat<T, B128>;
/// `Flat` with 256-byte buffer.
pub type Flat256<T> = Flat<T, B256>;

/// `Flex` with 16-byte buffer.
pub type Flex16<T> = Flex<T, B16>;
/// `Flex` with 32-byte buffer.
pub type Flex32<T> = Flex<T, B32>;
/// `Flex` with 48-byte buffer.
pub type Flex48<T> = Flex<T, B48>;
/// `Flex` with 64-byte buffer.
pub type Flex64<T> = Flex<T, B64>;
/// `Flex` with 128-byte buffer.
pub type Flex128<T> = Flex<T, B128>;
/// `Flex` with 256-byte buffer.
pub type Flex256<T> = Flex<T, B256>;

/// Constructs a [`Flat`] from a concrete value via unsizing coercion.
///
/// The return type must be annotated so the compiler performs unsizing
/// coercion (e.g., `Flat<dyn Trait, B32>`).
///
/// # Panics
///
/// Panics if the concrete value doesn't fit in the buffer (after
/// reserving 8 bytes for metadata on `?Sized` targets).
///
/// # Examples
///
/// ```
/// use nexus_smartptr::{flat, Flat, B32};
/// use core::fmt::Display;
///
/// let f: Flat<dyn Display, B32> = flat!(42_u32);
/// assert_eq!(format!("{}", &*f), "42");
/// ```
#[macro_export]
macro_rules! flat {
    ($val:expr) => {{
        let val = $val;
        let ptr: *const _ = &val;
        // SAFETY: ptr is a (possibly fat) pointer produced by unsizing
        // coercion at the call site. Its metadata corresponds to the
        // concrete type of val.
        unsafe { $crate::Flat::new_raw(val, ptr) }
    }};
}

/// Constructs a [`Flex`] from a concrete value via unsizing coercion.
///
/// Stores inline if the value fits, otherwise heap-allocates.
/// The return type must be annotated so the compiler performs unsizing
/// coercion (e.g., `Flex<dyn Trait, B32>`).
///
/// # Examples
///
/// ```
/// use nexus_smartptr::{flex, Flex, B32};
/// use core::fmt::Display;
///
/// let f: Flex<dyn Display, B32> = flex!(42_u32);
/// assert!(f.is_inline());
/// assert_eq!(format!("{}", &*f), "42");
/// ```
#[macro_export]
macro_rules! flex {
    ($val:expr) => {{
        let val = $val;
        let ptr: *const _ = &val;
        // SAFETY: same as flat! — ptr's metadata matches val's concrete type.
        unsafe { $crate::Flex::new_raw(val, ptr) }
    }};
}
