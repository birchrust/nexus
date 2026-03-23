//! String parsing and numeric conversions for `Decimal`.
//!
//! Parsing accumulates into i128 for uniform overflow handling across
//! all backing types, then narrows to the target type.
//!
//! Uses SWAR (SIMD Within A Register) to parse 8 ASCII digits in ~6
//! operations on any 64-bit platform (~1.3ns vs ~2.2ns scalar).

use crate::Decimal;
#[cfg(feature = "std")]
use crate::error::ConvertError;
use crate::error::ParseError;
use crate::pow10::pow10_i128;

/// Parse exactly 8 ASCII digit bytes into a u64 using SWAR.
///
/// Returns `None` if any byte is not an ASCII digit (`0x30..=0x39`).
/// Uses shift-and-mask with explicit pair/quad/final combination.
/// Portable — no SIMD intrinsics, works on all 64-bit platforms.
#[inline(always)]
fn parse_8_digits(bytes: &[u8; 8]) -> Option<u64> {
    let chunk = u64::from_le_bytes(*bytes);

    // Validate: all bytes must be ASCII digits (0x30..=0x39)
    let lower = chunk.wrapping_sub(0x3030_3030_3030_3030);
    let upper = chunk.wrapping_add(0x4646_4646_4646_4646);
    if (lower | upper) & 0x8080_8080_8080_8080 != 0 {
        return None;
    }

    // Mask to digit values (0-9 per byte). LE layout:
    // byte0=d1 (first char), byte1=d2, ..., byte7=d8 (last char)
    let d = chunk & 0x0f0f_0f0f_0f0f_0f0f;

    // Step 1: combine adjacent pairs → 4 × u16
    // d1*10+d2, d3*10+d4, d5*10+d6, d7*10+d8
    let lo = d & 0x00ff_00ff_00ff_00ff;
    let hi = (d >> 8) & 0x00ff_00ff_00ff_00ff;
    let pairs = lo * 10 + hi;

    // Step 2: combine pairs → 2 × u32
    let lo2 = pairs & 0x0000_ffff_0000_ffff;
    let hi2 = (pairs >> 16) & 0x0000_ffff_0000_ffff;
    let quads = lo2 * 100 + hi2;

    // Step 3: combine to single u64
    let lo3 = quads & 0x0000_0000_ffff_ffff;
    let hi3 = quads >> 32;
    Some(lo3 * 10000 + hi3)
}

/// Parse a byte slice of ASCII digits into u64, using SWAR for ≥8 digits.
///
/// Returns `Err(Overflow)` if the value exceeds u64. Callers that need
/// wider results should fall back to `parse_digits_wide`.
#[inline]
fn parse_digits_u64(bytes: &[u8]) -> Result<u64, ParseError> {
    let mut result: u64 = 0;
    let mut pos = 0;

    // SWAR fast path: 8 digits at a time
    while pos + 8 <= bytes.len() {
        let chunk: &[u8; 8] = bytes[pos..pos + 8].try_into().unwrap();
        let val = parse_8_digits(chunk).ok_or(ParseError::InvalidFormat)?;
        result = result
            .checked_mul(100_000_000)
            .and_then(|v| v.checked_add(val))
            .ok_or(ParseError::Overflow)?;
        pos += 8;
    }

    // Scalar tail
    for &b in &bytes[pos..] {
        let digit = b.wrapping_sub(b'0');
        if digit > 9 {
            return Err(ParseError::InvalidFormat);
        }
        result = result
            .checked_mul(10)
            .and_then(|v| v.checked_add(digit as u64))
            .ok_or(ParseError::Overflow)?;
    }

    Ok(result)
}

/// Wide fallback: parse into i128 for strings that overflow u64 (>18 digits).
#[inline]
fn parse_digits_wide(bytes: &[u8]) -> Result<i128, ParseError> {
    let mut result: i128 = 0;
    let mut pos = 0;

    while pos + 8 <= bytes.len() {
        let chunk: &[u8; 8] = bytes[pos..pos + 8].try_into().unwrap();
        let val = parse_8_digits(chunk).ok_or(ParseError::InvalidFormat)?;
        result = result
            .checked_mul(100_000_000)
            .and_then(|v| v.checked_add(val as i128))
            .ok_or(ParseError::Overflow)?;
        pos += 8;
    }

    for &b in &bytes[pos..] {
        let digit = b.wrapping_sub(b'0');
        if digit > 9 {
            return Err(ParseError::InvalidFormat);
        }
        result = result
            .checked_mul(10)
            .and_then(|v| v.checked_add(digit as i128))
            .ok_or(ParseError::Overflow)?;
    }

    Ok(result)
}

macro_rules! impl_decimal_convert {
    ($backing:ty, $unsigned:ty) => {
        impl<const D: u8> Decimal<$backing, D> {
            // ========================================================
            // String parsing
            // ========================================================

            /// Parses a decimal string exactly. Rejects inputs with more
            /// fractional digits than `DECIMALS`.
            ///
            /// # Examples
            ///
            /// ```
            /// use nexus_decimal::Decimal;
            /// type D64 = Decimal<i64, 8>;
            ///
            /// let price = D64::from_str_exact("123.45").unwrap();
            /// assert_eq!(price, D64::new(123, 45_000_000));
            /// ```
            pub fn from_str_exact(s: &str) -> Result<Self, ParseError> {
                Self::parse_str(s.as_bytes(), false)
            }

            /// Parses a decimal string, rounding excess precision using
            /// banker's rounding (round half to even).
            ///
            /// # Examples
            ///
            /// ```
            /// use nexus_decimal::Decimal;
            /// type D64 = Decimal<i64, 8>;
            ///
            /// // Input has 10 decimal places, D64 has 8 — rounds
            /// let price = D64::from_str_lossy("1.2345678951").unwrap();
            /// assert_eq!(price, D64::new(1, 23_456_790)); // rounded up
            /// ```
            pub fn from_str_lossy(s: &str) -> Result<Self, ParseError> {
                Self::parse_str(s.as_bytes(), true)
            }

            /// Parses from a UTF-8 byte slice.
            pub fn from_utf8_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
                Self::parse_str(bytes, false)
            }

            fn parse_str(bytes: &[u8], lossy: bool) -> Result<Self, ParseError> {
                if bytes.is_empty() {
                    return Err(ParseError::InvalidFormat);
                }

                // Sign
                let (negative, start) = match bytes[0] {
                    b'-' => (true, 1),
                    b'+' => (false, 1),
                    _ => (false, 0),
                };

                if start >= bytes.len() {
                    return Err(ParseError::InvalidFormat);
                }

                // Find decimal point
                let dot_pos = bytes[start..].iter().position(|&b| b == b'.');

                let (int_bytes, frac_bytes) = match dot_pos {
                    Some(pos) => (&bytes[start..start + pos], &bytes[start + pos + 1..]),
                    None => (&bytes[start..], &b""[..]),
                };

                // Must have at least one digit somewhere
                if int_bytes.is_empty() && frac_bytes.is_empty() {
                    return Err(ParseError::InvalidFormat);
                }

                // Parse integer part — u64 fast path, i128 fallback
                let integer_u64 = parse_digits_u64(int_bytes);
                let scaled_integer: i128 = match integer_u64 {
                    Ok(v) => {
                        // Fast path: u64 → i128 widen, then scale
                        (v as i128)
                            .checked_mul(Self::SCALE as i128)
                            .ok_or(ParseError::Overflow)?
                    }
                    Err(ParseError::Overflow) => {
                        // Integer part > u64 — fall back to i128
                        let wide = parse_digits_wide(int_bytes)?;
                        wide.checked_mul(Self::SCALE as i128)
                            .ok_or(ParseError::Overflow)?
                    }
                    Err(e) => return Err(e),
                };

                // Parse fractional part
                let frac_len = frac_bytes.len();
                let d = D as usize;

                if !lossy && frac_len > d {
                    return Err(ParseError::PrecisionLoss);
                }

                // Parse up to D digits — u64 is always sufficient
                // (max D=38 digits, but parsed digits ≤ D which for
                // i64 backing is ≤18, fitting u64 easily)
                let parse_len = frac_len.min(d);
                let mut frac_value: i128 = match parse_digits_u64(&frac_bytes[..parse_len]) {
                    Ok(v) => v as i128,
                    Err(ParseError::Overflow) => parse_digits_wide(&frac_bytes[..parse_len])?,
                    Err(e) => return Err(e),
                };

                // Validate remaining digits are actual digits (even if not used)
                for &b in &frac_bytes[parse_len..] {
                    let digit = b.wrapping_sub(b'0');
                    if digit > 9 {
                        return Err(ParseError::InvalidFormat);
                    }
                }

                // Scale fractional value to fill remaining decimal places
                if parse_len < d {
                    let fill_scale = pow10_i128((d - parse_len) as u8);
                    frac_value *= fill_scale;
                }

                // Banker's rounding for lossy mode
                if lossy && frac_len > d {
                    let rounding_digit = frac_bytes[d].wrapping_sub(b'0');

                    let round_up = if rounding_digit > 5 {
                        true
                    } else if rounding_digit < 5 {
                        false
                    } else {
                        // Exactly 5 — check subsequent digits
                        let has_trailing = frac_bytes[d + 1..].iter().any(|&b| b != b'0');
                        if has_trailing {
                            true // > 0.5, round up
                        } else {
                            // Exactly 0.5 — banker's: round to even
                            frac_value % 2 != 0
                        }
                    };

                    if round_up {
                        frac_value += 1;
                        // Handle carry: if frac_value == SCALE, roll into integer
                        let scale_i128 = Self::SCALE as i128;
                        if frac_value >= scale_i128 {
                            frac_value -= scale_i128;
                            let carry = scale_i128;
                            let new_scaled = scaled_integer
                                .checked_add(carry)
                                .ok_or(ParseError::Overflow)?;
                            let abs_value = new_scaled
                                .checked_add(frac_value)
                                .ok_or(ParseError::Overflow)?;
                            let value = if negative {
                                abs_value.checked_neg().ok_or(ParseError::Overflow)?
                            } else {
                                abs_value
                            };
                            return Self::narrow(value);
                        }
                    }
                }

                // Combine
                let abs_value = scaled_integer
                    .checked_add(frac_value)
                    .ok_or(ParseError::Overflow)?;

                let value = if negative {
                    abs_value.checked_neg().ok_or(ParseError::Overflow)?
                } else {
                    abs_value
                };

                Self::narrow(value)
            }

            /// Narrow an i128 value to the backing type, returning ParseError::Overflow
            /// if it doesn't fit.
            #[inline]
            fn narrow(value: i128) -> Result<Self, ParseError> {
                if value > <$backing>::MAX as i128 || value < <$backing>::MIN as i128 {
                    Err(ParseError::Overflow)
                } else {
                    Ok(Self {
                        value: value as $backing,
                    })
                }
            }

            // ========================================================
            // Integer conversions
            // ========================================================

            /// Creates a `Decimal` from an `i32`. Returns `None` on overflow.
            #[inline]
            pub const fn from_i32(value: i32) -> Option<Self> {
                let scaled = (value as i128).checked_mul(Self::SCALE as i128);
                match scaled {
                    Some(v) if v >= <$backing>::MIN as i128 && v <= <$backing>::MAX as i128 => {
                        Some(Self {
                            value: v as $backing,
                        })
                    }
                    _ => None,
                }
            }

            /// Creates a `Decimal` from an `i64`. Returns `None` on overflow.
            #[inline]
            pub const fn from_i64(value: i64) -> Option<Self> {
                let scaled = (value as i128).checked_mul(Self::SCALE as i128);
                match scaled {
                    Some(v) if v >= <$backing>::MIN as i128 && v <= <$backing>::MAX as i128 => {
                        Some(Self {
                            value: v as $backing,
                        })
                    }
                    _ => None,
                }
            }

            /// Creates a `Decimal` from a `u32`. Returns `None` on overflow.
            #[inline]
            pub const fn from_u32(value: u32) -> Option<Self> {
                Self::from_i64(value as i64)
            }

            /// Creates a `Decimal` from a `u64`. Returns `None` on overflow.
            #[inline]
            pub const fn from_u64(value: u64) -> Option<Self> {
                if value > i64::MAX as u64 {
                    // Could overflow i128 multiplication for large SCALE
                    let scaled = (value as i128).checked_mul(Self::SCALE as i128);
                    match scaled {
                        Some(v) if v <= <$backing>::MAX as i128 => Some(Self {
                            value: v as $backing,
                        }),
                        _ => None,
                    }
                } else {
                    Self::from_i64(value as i64)
                }
            }

            // ========================================================
            // Float conversions
            // ========================================================

            /// Converts to `f64`. Exact for values with ≤15 significant digits.
            #[inline]
            pub fn to_f64(self) -> f64 {
                let scale = Self::SCALE as f64;
                let integer = (self.value / Self::SCALE) as f64;
                let frac = (self.value % Self::SCALE) as f64 / scale;
                integer + frac
            }

            /// Converts to `f32`.
            #[inline]
            pub fn to_f32(self) -> f32 {
                self.to_f64() as f32
            }

            /// Creates a `Decimal` from an `f64`. Returns error on NaN, Inf, or overflow.
            ///
            /// Requires the `std` feature (uses `f64::round()`).
            #[cfg(feature = "std")]
            #[inline]
            pub fn from_f64(value: f64) -> Result<Self, ConvertError> {
                if !value.is_finite() {
                    return Err(ConvertError::Overflow);
                }

                let scaled = value * (Self::SCALE as f64);

                // Bounds check (f64 comparison is safe for this range)
                if scaled > <$backing>::MAX as f64 || scaled < <$backing>::MIN as f64 {
                    return Err(ConvertError::Overflow);
                }

                Ok(Self {
                    value: scaled.round() as $backing,
                })
            }

            /// Creates a `Decimal` from an `f32`.
            ///
            /// Requires the `std` feature (uses `f64::round()`).
            #[cfg(feature = "std")]
            #[inline]
            pub fn from_f32(value: f32) -> Result<Self, ConvertError> {
                Self::from_f64(value as f64)
            }
        }
    };
}

impl_decimal_convert!(i32, u32);
impl_decimal_convert!(i64, u64);
impl_decimal_convert!(i128, u128);
