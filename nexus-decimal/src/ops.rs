//! Operator trait implementations for `Decimal`.
//!
//! Panicking semantics matching std integers: overflow panics in debug,
//! wraps in release. Use `checked_*` for explicit fallibility.
//!
//! Operators panic on overflow, matching std integer behavior.

use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use crate::Decimal;

macro_rules! impl_decimal_ops {
    ($backing:ty) => {
        impl<const D: u8> Add for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn add(self, rhs: Self) -> Self {
                match self.checked_add(rhs) {
                    Some(v) => v,
                    None => panic!("decimal addition overflow"),
                }
            }
        }

        impl<const D: u8> Sub for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn sub(self, rhs: Self) -> Self {
                match self.checked_sub(rhs) {
                    Some(v) => v,
                    None => panic!("decimal subtraction overflow"),
                }
            }
        }

        impl<const D: u8> Neg for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn neg(self) -> Self {
                match self.checked_neg() {
                    Some(v) => v,
                    None => panic!("decimal negation overflow"),
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
                    None => panic!("decimal multiplication overflow"),
                }
            }
        }

        impl<const D: u8> Div for Decimal<$backing, D> {
            type Output = Self;

            #[inline(always)]
            fn div(self, rhs: Self) -> Self {
                match self.checked_div(rhs) {
                    Some(v) => v,
                    None => panic!("decimal division overflow or division by zero"),
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
    };
}

impl_decimal_mul_div_ops!(i32);
impl_decimal_mul_div_ops!(i64);
impl_decimal_mul_div_ops!(i128);
