//! Named constants for common decimal values.
//!
//! Core constants (`ZERO`, `ONE`, `MAX`, `MIN`) are generated for each
//! backing type. Financial constants (CENT, BASIS_POINT, etc.) are
//! deferred to Phase 4 (`financial.rs`) since they require SCALE
//! divisibility checks.

use crate::Decimal;

macro_rules! impl_decimal_constants {
    ($backing:ty) => {
        impl<const D: u8> Decimal<$backing, D> {
            /// Zero (`0`).
            pub const ZERO: Self = Self { value: 0 };

            /// One (`1.0`).
            pub const ONE: Self = Self { value: Self::SCALE };

            /// Negative one (`-1.0`).
            pub const NEG_ONE: Self = Self {
                value: -Self::SCALE,
            };

            /// Maximum representable value.
            pub const MAX: Self = Self {
                value: <$backing>::MAX,
            };

            /// Minimum representable value.
            pub const MIN: Self = Self {
                value: <$backing>::MIN,
            };
        }
    };
}

impl_decimal_constants!(i32);
impl_decimal_constants!(i64);
impl_decimal_constants!(i128);
