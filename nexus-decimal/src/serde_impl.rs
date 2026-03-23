//! Serde integration for `Decimal` (feature = "serde").
//!
//! - Human-readable (JSON, TOML): serializes as decimal string
//! - Binary (bincode, MessagePack): serializes as raw backing integer

use core::fmt;
use core::marker::PhantomData;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Decimal;

macro_rules! impl_decimal_serde {
    ($backing:ty) => {
        impl<const D: u8> Serialize for Decimal<$backing, D> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                if serializer.is_human_readable() {
                    // Stack buffer — zero heap allocation
                    let mut buf = [0u8; 64];
                    let len = self.write_to_buf(&mut buf);
                    // SAFETY: write_to_buf only writes ASCII
                    let s = unsafe { core::str::from_utf8_unchecked(&buf[..len]) };
                    serializer.serialize_str(s)
                } else {
                    self.value.serialize(serializer)
                }
            }
        }

        impl<'de, const D: u8> Deserialize<'de> for Decimal<$backing, D> {
            fn deserialize<De: Deserializer<'de>>(deserializer: De) -> Result<Self, De::Error> {
                if deserializer.is_human_readable() {
                    deserializer.deserialize_str(DecimalVisitor::<$backing, D>(PhantomData))
                } else {
                    let value = <$backing>::deserialize(deserializer)?;
                    Ok(Self { value })
                }
            }
        }

        impl<const D: u8> Visitor<'_> for DecimalVisitor<$backing, D> {
            type Value = Decimal<$backing, D>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a decimal number string")
            }

            fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
                Decimal::<$backing, D>::from_str_exact(s).map_err(de::Error::custom)
            }
        }
    };
}

struct DecimalVisitor<B, const D: u8>(PhantomData<B>);

impl_decimal_serde!(i32);
impl_decimal_serde!(i64);
impl_decimal_serde!(i128);
