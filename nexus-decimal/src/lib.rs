//! Fixed-point decimal arithmetic with compile-time precision.
//!
//! `nexus-decimal` provides [`Decimal<B, DECIMALS>`] — a generic
//! fixed-point type parameterized by backing integer and decimal
//! places. Operations are `const fn` where possible, zero-allocation,
//! and optimized for financial workloads.
//!
//! # Type Aliases
//!
//! | Alias | Backing | Decimals | Range | Use case |
//! |-------|---------|----------|-------|----------|
//! | [`D32`] | `i32` | 4 | ±214K | Embedded, space-constrained |
//! | [`D64`] | `i64` | 8 | ±92B | Traditional finance |
//! | [`D96`] | `i128` | 12 | ±39T | Cryptocurrency, DeFi |
//! | [`D128`] | `i128` | 18 | ±170B | Full i128 precision |
//!
//! Custom combinations work too — `Decimal<i64, 2>` for USD cents.
//!
//! # Examples
//!
//! ```
//! use nexus_decimal::D64;
//!
//! // Compile-time constants
//! const PRICE: D64 = D64::new(100, 50_000_000); // 100.50
//! const FEE: D64 = D64::from_raw(500_000);       // 0.005
//!
//! // Checked arithmetic
//! let total = PRICE.checked_add(FEE).unwrap();
//! assert_eq!(total.to_raw(), 10_050_500_000);
//!
//! // Rounding
//! let rounded = D64::new(1, 55_000_000).round(); // 1.55 → 2.0
//! assert_eq!(rounded.to_raw(), D64::new(2, 0).to_raw());
//! ```
//!
//! # Arithmetic Variants
//!
//! Every arithmetic operation comes in four flavors:
//!
//! | Variant | Returns | On overflow |
//! |---------|---------|-------------|
//! | `checked_*` | `Option<Self>` | `None` |
//! | `try_*` | `Result<Self, OverflowError>` | `Err(OverflowError)` |
//! | `saturating_*` | `Self` | Clamps to `MIN`/`MAX` |
//! | `wrapping_*` | `Self` | Wraps around |
//!
//! Operators (`+`, `-`, `-x`) panic on overflow, matching std integer
//! behavior. Use `checked_*` or `try_*` for fallible paths.

#![no_std]

#[cfg(feature = "std")]
extern crate std;

pub mod backing;
pub mod error;

mod arithmetic;
mod constants;
mod decimal;
mod ops;
mod pow10;
mod reciprocal;
mod rounding;
mod wide;

pub use backing::Backing;
pub use decimal::Decimal;
pub use error::{ConvertError, DivError, OverflowError, ParseError};

/// 32-bit decimal with 4 fractional digits. Range: ±214,748.3647
pub type D32 = Decimal<i32, 4>;

/// 64-bit decimal with 8 fractional digits. Range: ±92,233,720,368.54775807
pub type D64 = Decimal<i64, 8>;

/// 96-bit decimal with 12 fractional digits. Range: ±39,614,081,257,132.168796771975
pub type D96 = Decimal<i128, 12>;

/// 128-bit decimal with 18 fractional digits.
pub type D128 = Decimal<i128, 18>;
