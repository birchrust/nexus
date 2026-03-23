//! num-traits integration for `Decimal` (feature = "num-traits").
//!
//! Implements Zero, One, Num, Signed, Bounded, CheckedAdd/Sub/Mul/Div,
//! ToPrimitive, FromPrimitive. This unlocks generic numeric code.

use core::str::FromStr;

use num_traits::{
    Bounded, CheckedAdd, CheckedDiv, CheckedMul, CheckedNeg, CheckedSub, FromPrimitive, Num, One,
    Signed, ToPrimitive, Zero,
};

use crate::Decimal;

macro_rules! impl_num_traits {
    ($backing:ty) => {
        impl<const D: u8> Zero for Decimal<$backing, D> {
            #[inline]
            fn zero() -> Self {
                Self::ZERO
            }

            #[inline]
            fn is_zero(&self) -> bool {
                self.value == 0
            }
        }

        impl<const D: u8> One for Decimal<$backing, D> {
            #[inline]
            fn one() -> Self {
                Self::ONE
            }

            #[inline]
            fn is_one(&self) -> bool {
                self.value == Self::SCALE
            }
        }

        impl<const D: u8> Bounded for Decimal<$backing, D> {
            #[inline]
            fn min_value() -> Self {
                Self::MIN
            }

            #[inline]
            fn max_value() -> Self {
                Self::MAX
            }
        }

        impl<const D: u8> Num for Decimal<$backing, D> {
            type FromStrRadixErr = crate::error::ParseError;

            fn from_str_radix(str: &str, radix: u32) -> Result<Self, Self::FromStrRadixErr> {
                if radix != 10 {
                    return Err(crate::error::ParseError::InvalidFormat);
                }
                Self::from_str(str)
            }
        }

        impl<const D: u8> Signed for Decimal<$backing, D> {
            #[inline]
            fn abs(&self) -> Self {
                self.saturating_abs()
            }

            #[inline]
            fn abs_sub(&self, other: &Self) -> Self {
                if *self <= *other {
                    Self::ZERO
                } else {
                    *self - *other
                }
            }

            #[inline]
            fn signum(&self) -> Self {
                if self.value > 0 {
                    Self::ONE
                } else if self.value < 0 {
                    Self::NEG_ONE
                } else {
                    Self::ZERO
                }
            }

            #[inline]
            fn is_positive(&self) -> bool {
                self.value > 0
            }

            #[inline]
            fn is_negative(&self) -> bool {
                self.value < 0
            }
        }

        impl<const D: u8> CheckedAdd for Decimal<$backing, D> {
            #[inline]
            fn checked_add(&self, v: &Self) -> Option<Self> {
                Self::checked_add(*self, *v)
            }
        }

        impl<const D: u8> CheckedSub for Decimal<$backing, D> {
            #[inline]
            fn checked_sub(&self, v: &Self) -> Option<Self> {
                Self::checked_sub(*self, *v)
            }
        }

        impl<const D: u8> CheckedMul for Decimal<$backing, D> {
            #[inline]
            fn checked_mul(&self, v: &Self) -> Option<Self> {
                Self::checked_mul(*self, *v)
            }
        }

        impl<const D: u8> CheckedDiv for Decimal<$backing, D> {
            #[inline]
            fn checked_div(&self, v: &Self) -> Option<Self> {
                Self::checked_div(*self, *v)
            }
        }

        impl<const D: u8> CheckedNeg for Decimal<$backing, D> {
            #[inline]
            fn checked_neg(&self) -> Option<Self> {
                Self::checked_neg(*self)
            }
        }

        impl<const D: u8> ToPrimitive for Decimal<$backing, D> {
            #[inline]
            fn to_i64(&self) -> Option<i64> {
                let integer = self.to_integer();
                // Try to convert backing integer to i64
                let val = integer as i128;
                if val >= i64::MIN as i128 && val <= i64::MAX as i128 {
                    Some(val as i64)
                } else {
                    None
                }
            }

            #[inline]
            fn to_u64(&self) -> Option<u64> {
                let integer = self.to_integer();
                let val = integer as i128;
                if val >= 0 && val <= u64::MAX as i128 {
                    Some(val as u64)
                } else {
                    None
                }
            }

            #[inline]
            fn to_f64(&self) -> Option<f64> {
                Some(Self::to_f64(*self))
            }

            #[inline]
            fn to_f32(&self) -> Option<f32> {
                Some(Self::to_f32(*self))
            }
        }

        impl<const D: u8> FromPrimitive for Decimal<$backing, D> {
            #[inline]
            fn from_i64(n: i64) -> Option<Self> {
                Self::from_i64(n)
            }

            #[inline]
            fn from_u64(n: u64) -> Option<Self> {
                Self::from_u64(n)
            }

            #[inline]
            fn from_f64(n: f64) -> Option<Self> {
                Self::from_f64(n).ok()
            }

            #[inline]
            fn from_f32(n: f32) -> Option<Self> {
                Self::from_f32(n).ok()
            }

            #[inline]
            fn from_i32(n: i32) -> Option<Self> {
                Self::from_i32(n)
            }

            #[inline]
            fn from_u32(n: u32) -> Option<Self> {
                Self::from_u32(n)
            }
        }
    };
}

impl_num_traits!(i32);
impl_num_traits!(i64);
impl_num_traits!(i128);
