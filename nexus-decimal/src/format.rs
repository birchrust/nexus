//! Display, Debug, FromStr, and `write_to_buf` for `Decimal`.

use core::fmt;
use core::str::FromStr;

use crate::Decimal;
use crate::error::ParseError;

/// Lookup table for two-digit formatting (Alexandrescu/itoa pattern).
const DIGIT_PAIRS: &[u8; 200] = b"\
    00010203040506070809\
    10111213141516171819\
    20212223242526272829\
    30313233343536373839\
    40414243444546474849\
    50515253545556575859\
    60616263646566676869\
    70717273747576777879\
    80818283848586878889\
    90919293949596979899";

macro_rules! impl_decimal_format {
    ($backing:ty, $unsigned:ty) => {
        impl<const D: u8> Decimal<$backing, D> {
            /// Write decimal representation to a byte buffer.
            ///
            /// Returns the number of bytes written. Buffer must be at
            /// least 64 bytes. Useful for wire protocol encoding without
            /// `fmt` overhead.
            pub fn write_to_buf(&self, buf: &mut [u8]) -> usize {
                debug_assert!(buf.len() >= 64);

                if self.value == 0 {
                    buf[0] = b'0';
                    return 1;
                }

                let negative = self.value < 0;
                let abs = if negative {
                    self.value.unsigned_abs()
                } else {
                    self.value as $unsigned
                };

                let scale = Self::SCALE as $unsigned;
                let integer = abs / scale;
                let frac = abs % scale;

                let mut pos = 0;

                // Sign
                if negative {
                    buf[pos] = b'-';
                    pos += 1;
                }

                // Integer part → stack buffer using digit pairs, then copy
                let mut int_buf = [0u8; 40];
                let mut int_pos = 40;
                let mut val = integer;
                while val >= 100 {
                    int_pos -= 2;
                    let d = (val % 100) as usize * 2;
                    int_buf[int_pos] = DIGIT_PAIRS[d];
                    int_buf[int_pos + 1] = DIGIT_PAIRS[d + 1];
                    val /= 100;
                }
                if val >= 10 {
                    int_pos -= 2;
                    let d = val as usize * 2;
                    int_buf[int_pos] = DIGIT_PAIRS[d];
                    int_buf[int_pos + 1] = DIGIT_PAIRS[d + 1];
                } else {
                    int_pos -= 1;
                    int_buf[int_pos] = b'0' + val as u8;
                }
                let int_len = 40 - int_pos;
                buf[pos..pos + int_len].copy_from_slice(&int_buf[int_pos..40]);
                pos += int_len;

                // Fractional part
                if frac > 0 {
                    buf[pos] = b'.';
                    pos += 1;

                    let mut frac_buf = [b'0'; 40];
                    let mut frac_val = frac;
                    let mut frac_pos = D as usize;

                    while frac_val >= 100 && frac_pos >= 2 {
                        frac_pos -= 2;
                        let d = (frac_val % 100) as usize * 2;
                        frac_buf[frac_pos] = DIGIT_PAIRS[d];
                        frac_buf[frac_pos + 1] = DIGIT_PAIRS[d + 1];
                        frac_val /= 100;
                    }
                    while frac_val > 0 && frac_pos > 0 {
                        frac_pos -= 1;
                        frac_buf[frac_pos] = b'0' + (frac_val % 10) as u8;
                        frac_val /= 10;
                    }

                    // Strip trailing zeros
                    let mut end = D as usize;
                    while end > 0 && frac_buf[end - 1] == b'0' {
                        end -= 1;
                    }

                    buf[pos..pos + end].copy_from_slice(&frac_buf[..end]);
                    pos += end;
                }

                pos
            }
        }

        impl<const D: u8> fmt::Display for Decimal<$backing, D> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut buf = [0u8; 64];
                let len = self.write_to_buf(&mut buf);
                // SAFETY: write_to_buf only writes ASCII digits, '-', and '.'
                let s = unsafe { core::str::from_utf8_unchecked(&buf[..len]) };
                f.write_str(s)
            }
        }

        impl<const D: u8> fmt::Debug for Decimal<$backing, D> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if f.alternate() {
                    f.debug_struct("Decimal")
                        .field("value", &self.value)
                        .finish()
                } else {
                    write!(f, "{self}")
                }
            }
        }

        impl<const D: u8> FromStr for Decimal<$backing, D> {
            type Err = ParseError;

            #[inline]
            fn from_str(s: &str) -> Result<Self, ParseError> {
                Self::from_str_exact(s)
            }
        }
    };
}

impl_decimal_format!(i32, u32);
impl_decimal_format!(i64, u64);
impl_decimal_format!(i128, u128);
