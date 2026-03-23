//! Operator trait implementations for `Decimal`.
//!
//! Panicking semantics matching std integers: overflow panics in debug,
//! wraps in release. Use `checked_*` for explicit fallibility.
//!
//! Operators panic on overflow, matching std integer behavior.

use core::ops::{
    Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign,
};

use crate::Decimal;

#[cold]
#[inline(never)]
fn panic_overflow(op: &str) -> ! {
    panic!("decimal {op} overflow")
}

#[cold]
#[inline(never)]
fn panic_div_zero() -> ! {
    panic!("decimal division overflow or division by zero")
}

macro_rules! impl_decimal_ops {
    ($backing:ty) => {
        impl<const D: u8> Add for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn add(self, rhs: Self) -> Self {
                match self.checked_add(rhs) {
                    Some(v) => v,
                    None => panic_overflow("addition"),
                }
            }
        }

        impl<const D: u8> Sub for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn sub(self, rhs: Self) -> Self {
                match self.checked_sub(rhs) {
                    Some(v) => v,
                    None => panic_overflow("subtraction"),
                }
            }
        }

        impl<const D: u8> Neg for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn neg(self) -> Self {
                match self.checked_neg() {
                    Some(v) => v,
                    None => panic_overflow("negation"),
                }
            }
        }

        impl<const D: u8> AddAssign for Decimal<$backing, D> {
            #[inline(always)]
            fn add_assign(&mut self, rhs: Self) {
                *self = *self + rhs;
            }
        }

        impl<const D: u8> SubAssign for Decimal<$backing, D> {
            #[inline(always)]
            fn sub_assign(&mut self, rhs: Self) {
                *self = *self - rhs;
            }
        }
    };
}

impl_decimal_ops!(i32);
impl_decimal_ops!(i64);
impl_decimal_ops!(i128);

macro_rules! impl_decimal_mul_div_ops {
    ($backing:ty) => {
        impl<const D: u8> Mul for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn mul(self, rhs: Self) -> Self {
                match self.checked_mul(rhs) {
                    Some(v) => v,
                    None => panic_overflow("multiplication"),
                }
            }
        }

        impl<const D: u8> Div for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn div(self, rhs: Self) -> Self {
                match self.checked_div(rhs) {
                    Some(v) => v,
                    None => panic_div_zero(),
                }
            }
        }

        impl<const D: u8> MulAssign for Decimal<$backing, D> {
            #[inline(always)]
            fn mul_assign(&mut self, rhs: Self) {
                *self = *self * rhs;
            }
        }

        impl<const D: u8> DivAssign for Decimal<$backing, D> {
            #[inline(always)]
            fn div_assign(&mut self, rhs: Self) {
                *self = *self / rhs;
            }
        }

        impl<const D: u8> Rem for Decimal<$backing, D> {
            type Output = Self;

            /// Remainder after division. `self - (self / rhs) * rhs`.
            #[inline(always)]
            fn rem(self, rhs: Self) -> Self {
                Self {
                    value: self.value % rhs.value,
                }
            }
        }

        impl<const D: u8> RemAssign for Decimal<$backing, D> {
            #[inline(always)]
            fn rem_assign(&mut self, rhs: Self) {
                *self = *self % rhs;
            }
        }
    };
}

impl_decimal_mul_div_ops!(i32);
impl_decimal_mul_div_ops!(i64);
impl_decimal_mul_div_ops!(i128);

// ============================================================================
// Sum and Product iterator traits
// ============================================================================

macro_rules! impl_decimal_iter_traits {
    ($backing:ty) => {
        impl<const D: u8> core::iter::Sum for Decimal<$backing, D> {
            fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
                iter.fold(Self::ZERO, |acc, x| acc + x)
            }
        }

        impl<'a, const D: u8> core::iter::Sum<&'a Self> for Decimal<$backing, D> {
            fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
                iter.fold(Self::ZERO, |acc, &x| acc + x)
            }
        }

        impl<const D: u8> core::iter::Product for Decimal<$backing, D> {
            fn product<I: Iterator<Item = Self>>(iter: I) -> Self {
                iter.fold(Self::ONE, |acc, x| acc * x)
            }
        }

        impl<'a, const D: u8> core::iter::Product<&'a Self> for Decimal<$backing, D> {
            fn product<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
                iter.fold(Self::ONE, |acc, &x| acc * x)
            }
        }
    };
}

impl_decimal_iter_traits!(i32);
impl_decimal_iter_traits!(i64);
impl_decimal_iter_traits!(i128);

// ============================================================================
// From / TryFrom conversions
// ============================================================================

macro_rules! impl_decimal_from_traits {
    ($backing:ty) => {
        impl<const D: u8> TryFrom<i64> for Decimal<$backing, D> {
            type Error = crate::error::ConvertError;

            fn try_from(value: i64) -> Result<Self, Self::Error> {
                Self::from_i64(value).ok_or(crate::error::ConvertError::Overflow)
            }
        }

        impl<const D: u8> TryFrom<u64> for Decimal<$backing, D> {
            type Error = crate::error::ConvertError;

            fn try_from(value: u64) -> Result<Self, Self::Error> {
                Self::from_u64(value).ok_or(crate::error::ConvertError::Overflow)
            }
        }

        #[cfg(feature = "std")]
        impl<const D: u8> TryFrom<f64> for Decimal<$backing, D> {
            type Error = crate::error::ConvertError;

            fn try_from(value: f64) -> Result<Self, Self::Error> {
                Self::from_f64(value)
            }
        }

        #[cfg(feature = "std")]
        impl<const D: u8> TryFrom<f32> for Decimal<$backing, D> {
            type Error = crate::error::ConvertError;

            fn try_from(value: f32) -> Result<Self, Self::Error> {
                Self::from_f32(value)
            }
        }
    };
}

impl_decimal_from_traits!(i32);
impl_decimal_from_traits!(i64);
impl_decimal_from_traits!(i128);
