//! Integer parsing utilities for ASCII strings.
//!
//! This module provides macros and types for parsing integers from ASCII strings.

/// Macro to generate integer parsing methods for a type.
///
/// Usage: `impl_parse_int!(TypeName, as_str_method);`
macro_rules! impl_parse_int {
    ($ty:ty, $as_str:ident) => {
        impl $ty {
            /// Parses the string as a `u8`.
            ///
            /// # Errors
            ///
            /// Returns an error if the string is not a valid unsigned integer
            /// or overflows `u8`.
            ///
            /// # Example
            ///
            /// ```
            /// # use nexus_ascii::*;
            /// let s: AsciiString<32> = AsciiString::try_from("255")?;
            /// assert_eq!(s.parse_u8()?, 255u8);
            /// # Ok::<(), Box<dyn std::error::Error>>(())
            /// ```
            #[inline]
            pub fn parse_u8(&self) -> Result<u8, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `u16`.
            #[inline]
            pub fn parse_u16(&self) -> Result<u16, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `u32`.
            #[inline]
            pub fn parse_u32(&self) -> Result<u32, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `u64`.
            ///
            /// # Example
            ///
            /// ```
            /// # use nexus_ascii::*;
            /// let s: AsciiString<32> = AsciiString::try_from("12345678901234")?;
            /// assert_eq!(s.parse_u64()?, 12345678901234u64);
            /// # Ok::<(), Box<dyn std::error::Error>>(())
            /// ```
            #[inline]
            pub fn parse_u64(&self) -> Result<u64, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `u128`.
            #[inline]
            pub fn parse_u128(&self) -> Result<u128, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `usize`.
            #[inline]
            pub fn parse_usize(&self) -> Result<usize, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i8`.
            ///
            /// # Example
            ///
            /// ```
            /// # use nexus_ascii::*;
            /// let s: AsciiString<32> = AsciiString::try_from("-128")?;
            /// assert_eq!(s.parse_i8()?, -128i8);
            /// # Ok::<(), Box<dyn std::error::Error>>(())
            /// ```
            #[inline]
            pub fn parse_i8(&self) -> Result<i8, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i16`.
            #[inline]
            pub fn parse_i16(&self) -> Result<i16, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i32`.
            #[inline]
            pub fn parse_i32(&self) -> Result<i32, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i64`.
            ///
            /// # Example
            ///
            /// ```
            /// # use nexus_ascii::*;
            /// let s: AsciiString<32> = AsciiString::try_from("-12345678901234")?;
            /// assert_eq!(s.parse_i64()?, -12345678901234i64);
            /// # Ok::<(), Box<dyn std::error::Error>>(())
            /// ```
            #[inline]
            pub fn parse_i64(&self) -> Result<i64, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i128`.
            #[inline]
            pub fn parse_i128(&self) -> Result<i128, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `isize`.
            #[inline]
            pub fn parse_isize(&self) -> Result<isize, core::num::ParseIntError> {
                self.$as_str().parse()
            }
        }
    };
}

/// Macro to generate integer parsing methods for a generic type with const CAP.
macro_rules! impl_parse_int_generic {
    ($ty:ident, $as_str:ident) => {
        impl<const CAP: usize> $ty<CAP> {
            /// Parses the string as a `u8`.
            ///
            /// # Errors
            ///
            /// Returns an error if the string is not a valid unsigned integer
            /// or overflows `u8`.
            #[inline]
            pub fn parse_u8(&self) -> Result<u8, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `u16`.
            #[inline]
            pub fn parse_u16(&self) -> Result<u16, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `u32`.
            #[inline]
            pub fn parse_u32(&self) -> Result<u32, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `u64`.
            ///
            /// # Example
            ///
            /// ```
            /// # use nexus_ascii::*;
            /// let s: AsciiString<32> = AsciiString::try_from("12345678901234")?;
            /// assert_eq!(s.parse_u64()?, 12345678901234u64);
            /// # Ok::<(), Box<dyn std::error::Error>>(())
            /// ```
            #[inline]
            pub fn parse_u64(&self) -> Result<u64, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `u128`.
            #[inline]
            pub fn parse_u128(&self) -> Result<u128, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as a `usize`.
            #[inline]
            pub fn parse_usize(&self) -> Result<usize, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i8`.
            #[inline]
            pub fn parse_i8(&self) -> Result<i8, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i16`.
            #[inline]
            pub fn parse_i16(&self) -> Result<i16, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i32`.
            #[inline]
            pub fn parse_i32(&self) -> Result<i32, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i64`.
            #[inline]
            pub fn parse_i64(&self) -> Result<i64, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `i128`.
            #[inline]
            pub fn parse_i128(&self) -> Result<i128, core::num::ParseIntError> {
                self.$as_str().parse()
            }

            /// Parses the string as an `isize`.
            #[inline]
            pub fn parse_isize(&self) -> Result<isize, core::num::ParseIntError> {
                self.$as_str().parse()
            }
        }
    };
}

pub(crate) use impl_parse_int;
pub(crate) use impl_parse_int_generic;
