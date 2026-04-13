//! Miri tests for nexus-slot conflation slot.
//!
//! Run: `cargo +nightly miri test -p nexus-slot --test miri_tests`

use nexus_slot::spsc;

#[test]
fn write_and_read() {
    let (mut writer, mut reader) = spsc::slot::<u64>();
    writer.write(42);
    assert_eq!(reader.read(), Some(42));
}

#[test]
fn overwrite_conflation() {
    // Use u64 (no padding) to avoid miri false positive from the
    // word-at-a-time atomic_store on structs with padding bytes.
    let (mut writer, mut reader) = spsc::slot::<u64>();

    writer.write(10);
    writer.write(20);
    writer.write(30);

    // Conflation: only the last written value is observed.
    assert_eq!(reader.read(), Some(30));
}

#[test]
fn read_before_write_returns_none() {
    let (_writer, mut reader) = spsc::slot::<u64>();
    assert_eq!(reader.read(), None);
}
