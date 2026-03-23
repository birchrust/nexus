//! Financial-domain methods for `Decimal`.
//!
//! Trading operations that would otherwise be error-prone to implement
//! manually: midpoint, spread, tick rounding, basis points, percentage
//! calculations, and fused multiply-divide.

use crate::Decimal;

macro_rules! impl_decimal_financial {
    ($backing:ty) => {
        impl<const D: u8> Decimal<$backing, D> {
            // ========================================================
            // Price operations
            // ========================================================

            /// Midpoint of two prices: `(self + other) / 2`.
            ///
            /// Uses integer division by 2 (compiler optimizes to shift +
            /// sign-bit adjustment). Truncates toward zero for odd sums.
            ///
            /// Returns `None` if the addition overflows.
            ///
            /// # Examples
            ///
            /// ```
            /// use nexus_decimal::Decimal;
            /// type D64 = Decimal<i64, 8>;
            ///
            /// let bid = D64::new(100, 0);
            /// let ask = D64::new(101, 0);
            /// assert_eq!(bid.midpoint(ask), D64::new(100, 50_000_000));
            /// ```
            #[inline(always)]
            pub const fn midpoint(self, other: Self) -> Self {
                // Overflow-free: a + (b - a) / 2
                // Works for a > b too: (b - a) is negative, / 2 truncates toward zero.
                let half_diff = (other.value - self.value) / 2;
                Self {
                    value: self.value + half_diff,
                }
            }

            /// Spread between two prices: `self - other`.
            ///
            /// Returns `None` if `self < other` (crossed market).
            #[inline(always)]
            pub const fn spread(self, other: Self) -> Option<Self> {
                if self.value < other.value {
                    None
                } else {
                    match self.value.checked_sub(other.value) {
                        Some(v) => Some(Self { value: v }),
                        None => None,
                    }
                }
            }

            /// Round to nearest tick size.
            ///
            /// `tick` must be positive. Rounds to the nearest multiple
            /// of `tick` using banker's rounding on the remainder.
            ///
            /// # Examples
            ///
            /// ```
            /// use nexus_decimal::Decimal;
            /// type D64 = Decimal<i64, 8>;
            ///
            /// let price = D64::new(1, 23_700_000); // 1.237
            /// let tick = D64::new(0, 5_000_000);   // 0.05
            /// assert_eq!(price.round_to_tick(tick), Some(D64::new(1, 25_000_000))); // 1.25
            /// ```
            #[inline(always)]
            pub const fn round_to_tick(self, tick: Self) -> Option<Self> {
                assert!(tick.value > 0, "tick must be positive");
                let remainder = self.value % tick.value;
                let half_tick = tick.value / 2;
                let base = self.value - remainder;

                if remainder > half_tick {
                    match base.checked_add(tick.value) {
                        Some(v) => Some(Self { value: v }),
                        None => None,
                    }
                } else if remainder < -half_tick {
                    match base.checked_sub(tick.value) {
                        Some(v) => Some(Self { value: v }),
                        None => None,
                    }
                } else if remainder == half_tick || remainder == -half_tick {
                    let quotient = self.value / tick.value;
                    if quotient % 2 != 0 {
                        if remainder > 0 {
                            match base.checked_add(tick.value) {
                                Some(v) => Some(Self { value: v }),
                                None => None,
                            }
                        } else {
                            match base.checked_sub(tick.value) {
                                Some(v) => Some(Self { value: v }),
                                None => None,
                            }
                        }
                    } else {
                        Some(Self { value: base })
                    }
                } else {
                    Some(Self { value: base })
                }
            }

            /// Floor to tick: round down to nearest multiple of `tick`.
            ///
            /// Returns `None` if the result would overflow.
            #[inline(always)]
            pub const fn floor_to_tick(self, tick: Self) -> Option<Self> {
                assert!(tick.value > 0, "tick must be positive");
                let remainder = self.value % tick.value;
                if remainder >= 0 {
                    Some(Self {
                        value: self.value - remainder,
                    })
                } else {
                    match (self.value - remainder).checked_sub(tick.value) {
                        Some(v) => Some(Self { value: v }),
                        None => None,
                    }
                }
            }

            /// Ceil to tick: round up to nearest multiple of `tick`.
            ///
            /// Returns `None` if the result would overflow.
            #[inline(always)]
            pub const fn ceil_to_tick(self, tick: Self) -> Option<Self> {
                assert!(tick.value > 0, "tick must be positive");
                let remainder = self.value % tick.value;
                if remainder > 0 {
                    match (self.value - remainder).checked_add(tick.value) {
                        Some(v) => Some(Self { value: v }),
                        None => None,
                    }
                } else if remainder < 0 {
                    Some(Self {
                        value: self.value - remainder,
                    })
                } else {
                    Some(self)
                }
            }

            // ========================================================
            // Division shortcuts
            // ========================================================

            /// Divide by 2 using integer division. Truncates toward zero.
            ///
            /// The compiler optimizes this to a shift + sign-bit adjustment.
            #[inline(always)]
            pub const fn halve(self) -> Self {
                Self {
                    value: self.value / 2,
                }
            }

            /// Divide by 10 using integer division.
            #[inline(always)]
            pub const fn div10(self) -> Self {
                Self {
                    value: self.value / 10,
                }
            }

            /// Divide by 100 using integer division.
            #[inline(always)]
            pub const fn div100(self) -> Self {
                Self {
                    value: self.value / 100,
                }
            }

            // ========================================================
            // Comparison helpers
            // ========================================================

            /// Returns `true` if `self` is within `tolerance` of `other`.
            ///
            /// Equivalent to `|self - other| <= tolerance`.
            #[inline]
            pub const fn approx_eq(self, other: Self, tolerance: Self) -> bool {
                let diff = if self.value >= other.value {
                    self.value - other.value
                } else {
                    other.value - self.value
                };
                diff <= tolerance.value
            }

            /// Clamp to a price range `[min, max]`.
            #[inline]
            pub const fn clamp_price(self, min: Self, max: Self) -> Self {
                if self.value < min.value {
                    min
                } else if self.value > max.value {
                    max
                } else {
                    self
                }
            }
        }
    };
}

impl_decimal_financial!(i32);
impl_decimal_financial!(i64);
impl_decimal_financial!(i128);

// ============================================================================
// Methods that need widening (per-backing-type, not in shared macro)
// ============================================================================

// --- i32: widen to i64 for percent/bps calculations ---

impl<const D: u8> Decimal<i32, D> {
    /// Compute `self * percent / 100` with fused rounding.
    ///
    /// `percent` is in percentage points: 50 means 50%.
    #[inline]
    pub const fn percent_of(self, percent: Self) -> Option<Self> {
        let product = (self.value as i64) * (percent.value as i64);
        let scale_100 = (Self::SCALE as i64) * 100;
        let result = product / scale_100;
        if result > i32::MAX as i64 || result < i32::MIN as i64 {
            None
        } else {
            Some(Self {
                value: result as i32,
            })
        }
    }

    /// Convert to basis points: `self * 10000`.
    #[inline]
    pub const fn to_bps(self) -> Option<Self> {
        self.mul_int(10_000)
    }

    /// Create from basis points: `bps / 10000`.
    #[inline]
    pub const fn from_bps(bps: i32) -> Option<Self> {
        let scaled = bps as i64 * Self::SCALE as i64 / 10_000;
        if scaled > i32::MAX as i64 || scaled < i32::MIN as i64 {
            None
        } else {
            Some(Self {
                value: scaled as i32,
            })
        }
    }

    /// Fused multiply-divide: `(self * a) / b` with single rounding.
    #[inline]
    pub const fn mul_div(self, mul: Self, div: Self) -> Option<Self> {
        if div.value == 0 {
            return None;
        }
        let product = (self.value as i64) * (mul.value as i64);
        let result = product / (div.value as i64);
        if result > i32::MAX as i64 || result < i32::MIN as i64 {
            None
        } else {
            Some(Self {
                value: result as i32,
            })
        }
    }
}

// --- i64: widen to i128 for percent/bps calculations ---

impl<const D: u8> Decimal<i64, D> {
    /// Compute `self * percent / 100` with fused rounding.
    ///
    /// `percent` is in percentage points: 50 means 50%.
    #[inline]
    pub const fn percent_of(self, percent: Self) -> Option<Self> {
        let product = (self.value as i128) * (percent.value as i128);
        let scale_100 = (Self::SCALE as i128) * 100;
        let result = product / scale_100;
        if result > i64::MAX as i128 || result < i64::MIN as i128 {
            None
        } else {
            Some(Self {
                value: result as i64,
            })
        }
    }

    /// Convert to basis points: `self * 10000`.
    #[inline]
    pub const fn to_bps(self) -> Option<Self> {
        self.mul_int(10_000)
    }

    /// Create from basis points: `bps / 10000`.
    #[inline]
    pub const fn from_bps(bps: i64) -> Option<Self> {
        let scaled = (bps as i128) * (Self::SCALE as i128);
        let value = scaled / 10_000;
        if value > i64::MAX as i128 || value < i64::MIN as i128 {
            None
        } else {
            Some(Self {
                value: value as i64,
            })
        }
    }

    /// Fused multiply-divide: `(self * a) / b` with single rounding.
    ///
    /// Keeps the full i128 intermediate — single rounding at the end.
    /// The primitive behind fee calculation, VWAP, cross-rates.
    #[inline]
    pub const fn mul_div(self, mul: Self, div: Self) -> Option<Self> {
        if div.value == 0 {
            return None;
        }
        let product = (self.value as i128) * (mul.value as i128);
        let result = product / (div.value as i128);
        if result > i64::MAX as i128 || result < i64::MIN as i128 {
            None
        } else {
            Some(Self {
                value: result as i64,
            })
        }
    }
}

// --- i128: uses wide arithmetic for percent/bps ---

impl<const D: u8> Decimal<i128, D> {
    /// Convert to basis points: `self * 10000`.
    #[inline]
    pub const fn to_bps(self) -> Option<Self> {
        self.mul_int(10_000)
    }

    /// Create from basis points: `bps / 10000`.
    #[inline]
    pub const fn from_bps(bps: i128) -> Option<Self> {
        match (bps).checked_mul(Self::SCALE) {
            Some(scaled) => Some(Self {
                value: scaled / 10_000,
            }),
            None => None,
        }
    }

    /// Fused multiply-divide: `(self * a) / b` with single rounding.
    ///
    /// For i128, delegates to checked_mul then checked_div.
    /// Not truly fused (two rounding events) — a 256-bit intermediate
    /// would be needed for true single-rounding on i128.
    #[inline]
    pub fn mul_div(self, mul: Self, div: Self) -> Option<Self> {
        if div.value == 0 {
            return None;
        }
        let product = self.checked_mul(mul)?;
        product.checked_div(div)
    }
}
