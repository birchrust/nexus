//! Checked, saturating, wrapping, and try arithmetic for `Decimal`.
//!
//! Add/Sub/Neg/Abs are shared via macro (Phase 1).
//! Mul/Div differ per backing type (Phase 2):
//! - i32: widen to i64, native division (LLVM optimizes)
//! - i64: widen to i128, reciprocal division (avoids `__divti3`)
//! - i128: 192-bit wide arithmetic (manual limb math)

use crate::Decimal;
use crate::error::{DivError, OverflowError};

// ============================================================================
// Add / Sub / Neg / Abs — shared across all backing types
// ============================================================================

macro_rules! impl_decimal_arithmetic {
    ($backing:ty) => {
        impl<const D: u8> Decimal<$backing, D> {
            // ========================================================
            // Checked
            // ========================================================

            /// Checked addition. Returns `None` on overflow.
            #[inline(always)]
            pub const fn checked_add(self, rhs: Self) -> Option<Self> {
                match self.value.checked_add(rhs.value) {
                    Some(v) => Some(Self { value: v }),
                    None => None,
                }
            }

            /// Checked subtraction. Returns `None` on overflow.
            #[inline(always)]
            pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
                match self.value.checked_sub(rhs.value) {
                    Some(v) => Some(Self { value: v }),
                    None => None,
                }
            }

            /// Checked negation. Returns `None` if `self == MIN`.
            #[inline(always)]
            pub const fn checked_neg(self) -> Option<Self> {
                match self.value.checked_neg() {
                    Some(v) => Some(Self { value: v }),
                    None => None,
                }
            }

            /// Checked absolute value. Returns `None` if `self == MIN`.
            #[inline(always)]
            pub const fn checked_abs(self) -> Option<Self> {
                if self.value >= 0 {
                    Some(self)
                } else {
                    self.checked_neg()
                }
            }

            // ========================================================
            // Saturating
            // ========================================================

            /// Saturating addition. Clamps to `MIN`/`MAX` on overflow.
            #[inline(always)]
            pub const fn saturating_add(self, rhs: Self) -> Self {
                Self {
                    value: self.value.saturating_add(rhs.value),
                }
            }

            /// Saturating subtraction.
            #[inline(always)]
            pub const fn saturating_sub(self, rhs: Self) -> Self {
                Self {
                    value: self.value.saturating_sub(rhs.value),
                }
            }

            /// Saturating negation.
            #[inline(always)]
            pub const fn saturating_neg(self) -> Self {
                Self {
                    value: self.value.saturating_neg(),
                }
            }

            /// Saturating absolute value.
            #[inline(always)]
            pub const fn saturating_abs(self) -> Self {
                Self {
                    value: self.value.saturating_abs(),
                }
            }

            // ========================================================
            // Wrapping
            // ========================================================

            /// Wrapping addition.
            #[inline(always)]
            pub const fn wrapping_add(self, rhs: Self) -> Self {
                Self {
                    value: self.value.wrapping_add(rhs.value),
                }
            }

            /// Wrapping subtraction.
            #[inline(always)]
            pub const fn wrapping_sub(self, rhs: Self) -> Self {
                Self {
                    value: self.value.wrapping_sub(rhs.value),
                }
            }

            /// Wrapping negation.
            #[inline(always)]
            pub const fn wrapping_neg(self) -> Self {
                Self {
                    value: self.value.wrapping_neg(),
                }
            }

            /// Wrapping absolute value.
            #[inline(always)]
            pub const fn wrapping_abs(self) -> Self {
                Self {
                    value: self.value.wrapping_abs(),
                }
            }

            // ========================================================
            // Try (Result-returning) — add/sub/neg/abs
            // ========================================================

            /// Addition returning `Result`.
            #[inline(always)]
            pub const fn try_add(self, rhs: Self) -> Result<Self, OverflowError> {
                match self.checked_add(rhs) {
                    Some(v) => Ok(v),
                    None => Err(OverflowError),
                }
            }

            /// Subtraction returning `Result`.
            #[inline(always)]
            pub const fn try_sub(self, rhs: Self) -> Result<Self, OverflowError> {
                match self.checked_sub(rhs) {
                    Some(v) => Ok(v),
                    None => Err(OverflowError),
                }
            }

            /// Negation returning `Result`.
            #[inline(always)]
            pub const fn try_neg(self) -> Result<Self, OverflowError> {
                match self.checked_neg() {
                    Some(v) => Ok(v),
                    None => Err(OverflowError),
                }
            }

            /// Absolute value returning `Result`.
            #[inline(always)]
            pub const fn try_abs(self) -> Result<Self, OverflowError> {
                match self.checked_abs() {
                    Some(v) => Ok(v),
                    None => Err(OverflowError),
                }
            }
        }
    };
}

impl_decimal_arithmetic!(i32);
impl_decimal_arithmetic!(i64);
impl_decimal_arithmetic!(i128);

// ============================================================================
// Mul / Div — i32 (widen to i64, native division)
// ============================================================================

impl<const D: u8> Decimal<i32, D> {
    /// Checked multiplication. Widens to i64, divides by SCALE.
    #[inline(always)]
    pub const fn checked_mul(self, rhs: Self) -> Option<Self> {
        // i32 * i32 always fits in i64 — no overflow possible
        let product = (self.value as i64) * (rhs.value as i64);
        let result = product / (Self::SCALE as i64);

        if result > i32::MAX as i64 || result < i32::MIN as i64 {
            None
        } else {
            Some(Self {
                value: result as i32,
            })
        }
    }

    /// Checked division. Returns `None` if `rhs` is zero or result overflows.
    #[inline(always)]
    pub const fn checked_div(self, rhs: Self) -> Option<Self> {
        if rhs.value == 0 {
            return None;
        }
        let a = self.value as i64;
        let b = rhs.value as i64;
        let result = (a * Self::SCALE as i64) / b;

        if result > i32::MAX as i64 || result < i32::MIN as i64 {
            None
        } else {
            Some(Self {
                value: result as i32,
            })
        }
    }

    /// Saturating multiplication.
    #[inline(always)]
    pub const fn saturating_mul(self, rhs: Self) -> Self {
        let product = (self.value as i64) * (rhs.value as i64);
        let result = product / (Self::SCALE as i64);

        if result > i32::MAX as i64 {
            Self::MAX
        } else if result < i32::MIN as i64 {
            Self::MIN
        } else {
            Self {
                value: result as i32,
            }
        }
    }

    /// Wrapping multiplication.
    #[inline(always)]
    pub const fn wrapping_mul(self, rhs: Self) -> Self {
        let product = (self.value as i64) * (rhs.value as i64);
        Self {
            value: (product / (Self::SCALE as i64)) as i32,
        }
    }

    /// Saturating division.
    #[inline(always)]
    pub const fn saturating_div(self, rhs: Self) -> Self {
        if rhs.value == 0 {
            return Self::ZERO;
        }
        match self.checked_div(rhs) {
            Some(v) => v,
            None => {
                if (self.value > 0) == (rhs.value > 0) {
                    Self::MAX
                } else {
                    Self::MIN
                }
            }
        }
    }

    /// Wrapping division.
    #[inline(always)]
    pub const fn wrapping_div(self, rhs: Self) -> Self {
        if rhs.value == 0 {
            return Self::ZERO;
        }
        let a = self.value as i64;
        let b = rhs.value as i64;
        Self {
            value: ((a * Self::SCALE as i64) / b) as i32,
        }
    }

    /// Multiply by a plain integer (no rescaling).
    #[inline(always)]
    pub const fn mul_int(self, rhs: i32) -> Option<Self> {
        match self.value.checked_mul(rhs) {
            Some(v) => Some(Self { value: v }),
            None => None,
        }
    }

    /// Fused multiply-add: `(self * mul) + add` with single rescaling.
    #[inline(always)]
    pub const fn mul_add(self, mul: Self, add: Self) -> Option<Self> {
        let product = (self.value as i64) * (mul.value as i64);
        let rescaled = product / (Self::SCALE as i64);
        let result = rescaled + (add.value as i64);

        if result > i32::MAX as i64 || result < i32::MIN as i64 {
            None
        } else {
            Some(Self {
                value: result as i32,
            })
        }
    }

    /// Multiplication returning `Result`.
    #[inline(always)]
    pub const fn try_mul(self, rhs: Self) -> Result<Self, OverflowError> {
        match self.checked_mul(rhs) {
            Some(v) => Ok(v),
            None => Err(OverflowError),
        }
    }

    /// Division returning `Result` with specific error.
    #[inline(always)]
    pub const fn try_div(self, rhs: Self) -> Result<Self, DivError> {
        if rhs.value == 0 {
            return Err(DivError::DivisionByZero);
        }
        match self.checked_div(rhs) {
            Some(v) => Ok(v),
            None => Err(DivError::Overflow),
        }
    }
}

// ============================================================================
// Mul / Div — i64 (widen to i128, native division)
// ============================================================================
//
// Uses native i128 division (calls __divti3 on x86). Correct for all inputs.
// Phase 5 will evaluate replacing with a proper 128-bit Barrett reduction
// (u128 reciprocal with 256-bit intermediate) where cargo-asm shows benefit.

impl<const D: u8> Decimal<i64, D> {
    /// Checked multiplication. Widens to i128, divides by SCALE.
    #[inline(always)]
    pub const fn checked_mul(self, rhs: Self) -> Option<Self> {
        let a = self.value as i128;
        let b = rhs.value as i128;

        let Some(product) = a.checked_mul(b) else {
            return None;
        };

        let result = product / (Self::SCALE as i128);

        if result > i64::MAX as i128 || result < i64::MIN as i128 {
            None
        } else {
            Some(Self {
                value: result as i64,
            })
        }
    }

    /// Checked division. Returns `None` if `rhs` is zero or result overflows.
    #[inline(always)]
    pub const fn checked_div(self, rhs: Self) -> Option<Self> {
        if rhs.value == 0 {
            return None;
        }
        let a = self.value as i128;
        let b = rhs.value as i128;
        let result = (a * Self::SCALE as i128) / b;

        if result > i64::MAX as i128 || result < i64::MIN as i128 {
            None
        } else {
            Some(Self {
                value: result as i64,
            })
        }
    }

    /// Saturating multiplication.
    #[inline(always)]
    pub const fn saturating_mul(self, rhs: Self) -> Self {
        let product = (self.value as i128) * (rhs.value as i128);
        let result = product / (Self::SCALE as i128);

        if result > i64::MAX as i128 {
            Self::MAX
        } else if result < i64::MIN as i128 {
            Self::MIN
        } else {
            Self {
                value: result as i64,
            }
        }
    }

    /// Wrapping multiplication.
    #[inline(always)]
    pub const fn wrapping_mul(self, rhs: Self) -> Self {
        let product = (self.value as i128).wrapping_mul(rhs.value as i128);
        Self {
            value: (product / (Self::SCALE as i128)) as i64,
        }
    }

    /// Saturating division.
    #[inline(always)]
    pub const fn saturating_div(self, rhs: Self) -> Self {
        if rhs.value == 0 {
            return Self::ZERO;
        }
        match self.checked_div(rhs) {
            Some(v) => v,
            None => {
                if (self.value > 0) == (rhs.value > 0) {
                    Self::MAX
                } else {
                    Self::MIN
                }
            }
        }
    }

    /// Wrapping division.
    #[inline(always)]
    pub const fn wrapping_div(self, rhs: Self) -> Self {
        if rhs.value == 0 {
            return Self::ZERO;
        }
        let a = self.value as i128;
        let b = rhs.value as i128;
        Self {
            value: ((a * Self::SCALE as i128) / b) as i64,
        }
    }

    /// Multiply by a plain integer (no rescaling).
    #[inline(always)]
    pub const fn mul_int(self, rhs: i64) -> Option<Self> {
        match self.value.checked_mul(rhs) {
            Some(v) => Some(Self { value: v }),
            None => None,
        }
    }

    /// Fused multiply-add: `(self * mul) + add` with single rescaling.
    #[inline(always)]
    pub const fn mul_add(self, mul: Self, add: Self) -> Option<Self> {
        let a = self.value as i128;
        let b = mul.value as i128;

        let Some(product) = a.checked_mul(b) else {
            return None;
        };

        let rescaled = product / (Self::SCALE as i128);

        let Some(result) = rescaled.checked_add(add.value as i128) else {
            return None;
        };

        if result > i64::MAX as i128 || result < i64::MIN as i128 {
            None
        } else {
            Some(Self {
                value: result as i64,
            })
        }
    }

    /// Multiplication returning `Result`.
    #[inline(always)]
    pub const fn try_mul(self, rhs: Self) -> Result<Self, OverflowError> {
        match self.checked_mul(rhs) {
            Some(v) => Ok(v),
            None => Err(OverflowError),
        }
    }

    /// Division returning `Result` with specific error.
    #[inline(always)]
    pub const fn try_div(self, rhs: Self) -> Result<Self, DivError> {
        if rhs.value == 0 {
            return Err(DivError::DivisionByZero);
        }
        match self.checked_div(rhs) {
            Some(v) => Ok(v),
            None => Err(DivError::Overflow),
        }
    }
}

// ============================================================================
// Mul / Div — i128 (192-bit wide arithmetic, NOT const fn)
// ============================================================================

use crate::wide;

impl<const D: u8> Decimal<i128, D> {
    /// Threshold for fast-path multiplication (both operands < 2^64).
    const FAST_MUL_THRESHOLD: u128 = 1u128 << 64;

    /// Checked multiplication using 192-bit wide arithmetic.
    #[inline(always)]
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        if self.value == 0 || rhs.value == 0 {
            return Some(Self::ZERO);
        }

        let result_negative = (self.value < 0) != (rhs.value < 0);
        let a = self.value.unsigned_abs();
        let b = rhs.value.unsigned_abs();

        // Fast path: both values fit in 64 bits → product fits in 128 bits
        if a < Self::FAST_MUL_THRESHOLD && b < Self::FAST_MUL_THRESHOLD {
            let product = a * b;
            let quotient = product / (Self::SCALE as u128);
            return Self::from_unsigned(quotient, result_negative);
        }

        // Slow path: 192-bit multiplication
        let (prod_low, prod_high) = wide::mul_wide(a, b);
        let quotient = wide::div_192_by_const(prod_low, prod_high, Self::SCALE as u128)?;
        Self::from_unsigned(quotient, result_negative)
    }

    /// Checked division using 192-bit wide arithmetic.
    #[inline(always)]
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        if rhs.value == 0 {
            return None;
        }
        if self.value == 0 {
            return Some(Self::ZERO);
        }

        let result_negative = (self.value < 0) != (rhs.value < 0);
        let a = self.value.unsigned_abs();
        let b = rhs.value.unsigned_abs();
        let scale = Self::SCALE as u128;

        // Widen: a * SCALE (can exceed 128 bits)
        let (prod_low, prod_high) = wide::mul_u128_by_small(a, scale);

        // Divide 192-bit by runtime divisor
        let quotient = wide::div_192_by_u128(prod_low, prod_high, b)?;
        Self::from_unsigned(quotient, result_negative)
    }

    /// Saturating multiplication.
    #[inline(always)]
    pub fn saturating_mul(self, rhs: Self) -> Self {
        self.checked_mul(rhs).unwrap_or({
            if (self.value > 0) == (rhs.value > 0) {
                Self::MAX
            } else {
                Self::MIN
            }
        })
    }

    /// Wrapping multiplication.
    #[inline(always)]
    pub fn wrapping_mul(self, rhs: Self) -> Self {
        if self.value == 0 || rhs.value == 0 {
            return Self::ZERO;
        }

        let result_negative = (self.value < 0) != (rhs.value < 0);
        let a = self.value.unsigned_abs();
        let b = rhs.value.unsigned_abs();

        let (prod_low, prod_high) = wide::mul_wide(a, b);
        let quotient = wide::div_192_by_const_wrapping(prod_low, prod_high, Self::SCALE as u128);

        if result_negative {
            Self {
                value: (quotient as i128).wrapping_neg(),
            }
        } else {
            Self {
                value: quotient as i128,
            }
        }
    }

    /// Saturating division.
    #[inline(always)]
    pub fn saturating_div(self, rhs: Self) -> Self {
        if rhs.value == 0 {
            return Self::ZERO;
        }
        self.checked_div(rhs).unwrap_or({
            if (self.value > 0) == (rhs.value > 0) {
                Self::MAX
            } else {
                Self::MIN
            }
        })
    }

    /// Wrapping division.
    #[inline(always)]
    pub fn wrapping_div(self, rhs: Self) -> Self {
        if rhs.value == 0 {
            return Self::ZERO;
        }

        let result_negative = (self.value < 0) != (rhs.value < 0);
        let a = self.value.unsigned_abs();
        let b = rhs.value.unsigned_abs();
        let scale = Self::SCALE as u128;

        let (prod_low, prod_high) = wide::mul_u128_by_small(a, scale);
        let quotient = wide::div_192_by_u128_wrapping(prod_low, prod_high, b);

        if result_negative {
            Self {
                value: (quotient as i128).wrapping_neg(),
            }
        } else {
            Self {
                value: quotient as i128,
            }
        }
    }

    /// Multiply by a plain integer (no rescaling).
    #[inline(always)]
    pub const fn mul_int(self, rhs: i128) -> Option<Self> {
        match self.value.checked_mul(rhs) {
            Some(v) => Some(Self { value: v }),
            None => None,
        }
    }

    /// Fused multiply-add: `(self * mul) + add` with single rescaling.
    #[inline(always)]
    pub fn mul_add(self, mul: Self, add: Self) -> Option<Self> {
        if self.value == 0 || mul.value == 0 {
            return Some(add);
        }

        let product = self.checked_mul(mul)?;
        product.checked_add(add)
    }

    /// Multiplication returning `Result`.
    #[inline(always)]
    pub fn try_mul(self, rhs: Self) -> Result<Self, OverflowError> {
        self.checked_mul(rhs).ok_or(OverflowError)
    }

    /// Division returning `Result` with specific error.
    #[inline(always)]
    pub fn try_div(self, rhs: Self) -> Result<Self, DivError> {
        if rhs.value == 0 {
            return Err(DivError::DivisionByZero);
        }
        self.checked_div(rhs).ok_or(DivError::Overflow)
    }

    /// Helper: convert unsigned quotient + sign to Decimal, with bounds check.
    #[inline(always)]
    fn from_unsigned(quotient: u128, negative: bool) -> Option<Self> {
        if negative {
            // i128::MIN.unsigned_abs() = i128::MAX + 1
            if quotient > (i128::MAX as u128) + 1 {
                return None;
            }
            Some(Self {
                value: -(quotient as i128),
            })
        } else {
            if quotient > i128::MAX as u128 {
                return None;
            }
            Some(Self {
                value: quotient as i128,
            })
        }
    }
}
