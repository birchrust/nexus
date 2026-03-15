//! Regression test for suspected LLVM field elision in bit-packed structs.
//!
//! A coworker observed that a bool flag set via the builder but never read
//! back via its accessor was missing from the packed integer. The hypothesis
//! is that LLVM eliminated the write as dead code.
//!
//! This test validates that all fields contribute to the packed value even
//! when their accessors are never called. Run in release mode to exercise
//! LLVM optimizations: `cargo test --release -p nexus-bits -- field_elision`

use nexus_bits::{IntEnum, bit_storage};

#[derive(IntEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Market {
    Spot = 0,
    Perp = 1,
    Future = 2,
}

#[bit_storage(repr = u64)]
pub struct InstrumentId {
    #[field(start = 0, len = 16)]
    symbol: u16,
    #[field(start = 16, len = 4)]
    market: Market,
    #[flag(20)]
    is_inverse: bool,
    #[field(start = 24, len = 8)]
    venue: u8,
}

#[test]
fn unread_flag_is_packed() {
    let id = InstrumentId::builder()
        .symbol(42)
        .market(Market::Perp)
        .is_inverse(true)
        .venue(7)
        .build()
        .unwrap();

    // Verify raw value has the flag bit set, WITHOUT calling is_inverse()
    let raw = id.raw();
    let expected = 0x2A_u64 | (1u64 << 16) | (1u64 << 20) | (7u64 << 24);
    assert_eq!(
        raw, expected,
        "flag bit missing from packed value — possible LLVM field elision"
    );
}

#[test]
fn unread_field_is_packed() {
    let id = InstrumentId::builder()
        .symbol(1000)
        .market(Market::Future)
        .is_inverse(false)
        .venue(99)
        .build()
        .unwrap();

    // Only read symbol — do NOT read market, is_inverse, or venue
    let _ = id.symbol();

    let raw = id.raw();
    let expected = 0x03E8_u64 | (2u64 << 16) | (99u64 << 24);
    assert_eq!(raw, expected, "unread fields missing from packed value");
}

#[test]
fn no_fields_read_all_packed() {
    let id = InstrumentId::builder()
        .symbol(0xFFFF)
        .market(Market::Perp)
        .is_inverse(true)
        .venue(0xFF)
        .build()
        .unwrap();

    // Read NOTHING — just check raw
    let raw = id.raw();
    let expected = 0xFFFFu64 | (1u64 << 16) | (1u64 << 20) | (0xFFu64 << 24);
    assert_eq!(raw, expected, "fields missing when no accessors called");
}
