//! Core `Decimal<B, D>` type definition and constructors.

use crate::backing::Backing;

/// Fixed-point decimal with compile-time backing type and precision.
///
/// `B` is the backing integer type (`i32`, `i64`, `i128`).
/// `DECIMALS` is the number of fractional digits. Any combination
/// where `10^DECIMALS` fits in `B` is valid — `Decimal<i64, 2>` for
/// USD or `Decimal<i64, 8>` for BTC without any macro invocation.
///
/// The scale factor `10^DECIMALS` is validated at compile time.
/// Invalid combinations (e.g., `Decimal<i32, 10>`) fail to compile
/// when any associated constant or method is used.
///
/// # Examples
///
/// ```
/// use nexus_decimal::D64;
///
/// const PRICE: D64 = D64::new(100, 50_000_000); // 100.50
/// const FEE: D64 = D64::from_raw(500_000);       // 0.005
/// const TOTAL: D64 = match PRICE.checked_add(FEE) {
///     Some(v) => v,
///     None => panic!("overflow"),
/// };
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Decimal<B: Backing, const DECIMALS: u8> {
    pub(crate) value: B,
}

/// Generates constructors and query methods for a concrete backing type.
macro_rules! impl_decimal_core {
    ($backing:ty, $pow10_fn:path, $max_exp:expr) => {
        impl<const D: u8> Decimal<$backing, D> {
            /// The scale factor `10^DECIMALS`.
            ///
            /// Validated at compile time: panics if `DECIMALS` is too
            /// large for the backing type.
            pub const SCALE: $backing = {
                assert!(
                    (D as u32) <= $max_exp,
                    "DECIMALS too large for backing type"
                );
                $pow10_fn(D)
            };

            /// The number of fractional digits.
            pub const DECIMALS: u8 = D;

            /// Creates a `Decimal` from a raw pre-scaled value.
            ///
            /// No validation — the caller is responsible for ensuring
            /// the value is in the expected scale.
            #[inline(always)]
            pub const fn from_raw(value: $backing) -> Self {
                Self { value }
            }

            /// Returns the raw internal value (scaled by `10^DECIMALS`).
            #[inline(always)]
            pub const fn to_raw(self) -> $backing {
                self.value
            }

            /// Creates a `Decimal` from integer and fractional parts.
            ///
            /// The fractional part must be non-negative and less than
            /// `SCALE`. For negative values, negate the integer part:
            /// `new(-123, 45_000_000)` → `-123.45` (for `DECIMALS=8`).
            ///
            /// # Panics
            ///
            /// Panics if the result overflows the backing type.
            ///
            /// # Examples
            ///
            /// ```
            /// use nexus_decimal::D64;
            ///
            /// const PRICE: D64 = D64::new(100, 50_000_000); // 100.50
            /// const NEG: D64 = D64::new(-50, 25_000_000);   // -50.25
            /// ```
            pub const fn new(integer: $backing, fractional: $backing) -> Self {
                let Some(scaled) = integer.checked_mul(Self::SCALE) else {
                    panic!("overflow in Decimal::new: integer part too large")
                };

                let value = if integer >= 0 {
                    let Some(v) = scaled.checked_add(fractional) else {
                        panic!("overflow in Decimal::new")
                    };
                    v
                } else {
                    let Some(v) = scaled.checked_sub(fractional) else {
                        panic!("overflow in Decimal::new")
                    };
                    v
                };

                Self { value }
            }

            /// Returns `true` if the value is zero.
            #[inline(always)]
            pub const fn is_zero(self) -> bool {
                self.value == 0
            }

            /// Returns `true` if the value is strictly positive.
            #[inline(always)]
            pub const fn is_positive(self) -> bool {
                self.value > 0
            }

            /// Returns `true` if the value is strictly negative.
            #[inline(always)]
            pub const fn is_negative(self) -> bool {
                self.value < 0
            }

            /// Returns the signum: `-1`, `0`, or `1`.
            #[inline(always)]
            pub const fn signum(self) -> $backing {
                self.value.signum()
            }
        }

        impl<const D: u8> Default for Decimal<$backing, D> {
            #[inline]
            fn default() -> Self {
                Self::ZERO
            }
        }
    };
}

use crate::pow10::{pow10_i32, pow10_i64, pow10_i128};

impl_decimal_core!(i32, pow10_i32, 9);
impl_decimal_core!(i64, pow10_i64, 18);
impl_decimal_core!(i128, pow10_i128, 38);
