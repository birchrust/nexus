//! Miri-specific tests for memory safety verification.
//!
//! Run with: `cargo +nightly miri test --test miri_tests -- -Zmiri-ignore-leaks`
//!
//! The `-Zmiri-ignore-leaks` flag is required because:
//! - Allocators are intentionally leaked (Box::leak for stable addresses)
//! - Leaked slots (via `slot.leak()`) are intentionally not freed
//!
//! These tests verify:
//! - No use-after-free
//! - No double-free
//! - No uninitialized memory access
//! - No invalid pointer arithmetic
//! - Correct drop ordering

use nexus_slab::Allocator;
use std::cell::Cell;

// =============================================================================
// Helper Types
// =============================================================================

thread_local! {
    static DROP_COUNT: Cell<usize> = const { Cell::new(0) };
}

pub struct DropTracker(u64);

impl Drop for DropTracker {
    fn drop(&mut self) {
        DROP_COUNT.with(|c| c.set(c.get() + 1));
    }
}

fn reset_drop_count() {
    DROP_COUNT.with(|c| c.set(0));
}

fn get_drop_count() -> usize {
    DROP_COUNT.with(|c| c.get())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZeroSized;

#[derive(Clone)]
pub struct Large {
    data: [u64; 128],
}

// =============================================================================
// Basic Memory Safety
// =============================================================================

#[test]
fn miri_bounded_basic() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(8).build();

    let slot = alloc.new_slot(42);
    assert_eq!(*slot, 42);
    drop(slot);

    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_unbounded_basic() {
    let alloc: Allocator<u64> = Allocator::builder()
        .unbounded()
        .chunk_capacity(4)
        .build();

    let slot = alloc.new_slot(42);
    assert_eq!(*slot, 42);
    drop(slot);

    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_multiple_inserts() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(8).build();

    let s1 = alloc.new_slot(1);
    let s2 = alloc.new_slot(2);
    let s3 = alloc.new_slot(3);

    assert_eq!(*s1, 1);
    assert_eq!(*s2, 2);
    assert_eq!(*s3, 3);
}

#[test]
fn miri_slot_deref_mut() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let mut slot = alloc.new_slot(42);
    *slot = 100;
    assert_eq!(*slot, 100);
}

#[test]
fn miri_slot_replace() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let mut slot = alloc.new_slot(1);
    let old = slot.replace(2);
    assert_eq!(old, 1);
    assert_eq!(*slot, 2);
}

#[test]
fn miri_slot_into_inner() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let value = slot.into_inner();
    assert_eq!(value, 42);
    assert_eq!(alloc.len(), 0);
}

// =============================================================================
// Drop Safety
// =============================================================================

#[test]
fn miri_drop_on_slot_drop() {
    reset_drop_count();

    let alloc: Allocator<DropTracker> = Allocator::builder().bounded(4).build();

    {
        let _slot = alloc.new_slot(DropTracker(1));
    }

    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_into_inner() {
    reset_drop_count();

    let alloc: Allocator<DropTracker> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(DropTracker(1));
    let value = slot.into_inner();
    assert_eq!(get_drop_count(), 0);

    drop(value);
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_replace() {
    reset_drop_count();

    let alloc: Allocator<DropTracker> = Allocator::builder().bounded(4).build();

    let mut slot = alloc.new_slot(DropTracker(1));
    let old = slot.replace(DropTracker(2));
    drop(old);
    assert_eq!(get_drop_count(), 1);

    drop(slot);
    assert_eq!(get_drop_count(), 2);
}

#[test]
fn miri_no_drop_after_leak() {
    reset_drop_count();

    let alloc: Allocator<DropTracker> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(DropTracker(1));
    let _key = slot.leak();

    assert_eq!(get_drop_count(), 0);
}

// =============================================================================
// Heap-Allocated Types
// =============================================================================

#[test]
fn miri_string_insert_drop() {
    let alloc: Allocator<String> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot("hello world".to_string());
    assert_eq!(*slot, "hello world");
    drop(slot);

    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_vec_insert_drop() {
    let alloc: Allocator<Vec<u64>> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(vec![1, 2, 3, 4, 5]);
    assert_eq!(slot.len(), 5);
    drop(slot);

    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_box_insert_drop() {
    let alloc: Allocator<Box<[u8; 1024]>> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(Box::new([0u8; 1024]));
    assert_eq!(slot.len(), 1024);
    drop(slot);

    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_string_into_inner() {
    let alloc: Allocator<String> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot("hello".to_string());
    let value = slot.into_inner();
    assert_eq!(value, "hello");
}

#[test]
fn miri_vec_replace() {
    let alloc: Allocator<Vec<u64>> = Allocator::builder().bounded(4).build();

    let mut slot = alloc.new_slot(vec![1, 2, 3]);
    let old = slot.replace(vec![4, 5, 6, 7]);

    assert_eq!(old, vec![1, 2, 3]);
    assert_eq!(*slot, vec![4, 5, 6, 7]);
}

// =============================================================================
// Slot Reuse
// =============================================================================

#[test]
fn miri_slot_reuse_bounded() {
    let alloc: Allocator<String> = Allocator::builder().bounded(2).build();

    // Fill
    let s1 = alloc.new_slot("one".to_string());
    let s2 = alloc.new_slot("two".to_string());

    let k1 = s1.key();
    let k2 = s2.key();

    // Free one
    drop(s1);

    // Reuse
    let s3 = alloc.new_slot("three".to_string());
    assert_eq!(*s3, "three");
    assert_eq!(s3.key(), k1); // Reused slot 1

    // Free other
    drop(s2);

    // Reuse again
    let s4 = alloc.new_slot("four".to_string());
    assert_eq!(*s4, "four");
    assert_eq!(s4.key(), k2); // Reused slot 2
}

#[test]
fn miri_slot_reuse_single() {
    let alloc: Allocator<String> = Allocator::builder().bounded(1).build();

    for i in 0..10 {
        let slot = alloc.new_slot(format!("value_{}", i));
        assert_eq!(*slot, format!("value_{}", i));
        assert_eq!(slot.key().index(), 0);
    }
}

// =============================================================================
// Unbounded Growth
// =============================================================================

#[test]
fn miri_unbounded_growth() {
    let alloc: Allocator<u64> = Allocator::builder()
        .unbounded()
        .chunk_capacity(4)
        .build();

    // Fill multiple chunks
    let slots: Vec<_> = (0..12).map(|i| alloc.new_slot(i)).collect();

    assert_eq!(alloc.len(), 12);
    assert!(alloc.capacity() >= 12);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(*slot.get(), i as u64);
    }
}

#[test]
fn miri_unbounded_string_growth() {
    let alloc: Allocator<String> = Allocator::builder()
        .unbounded()
        .chunk_capacity(4)
        .build();

    let slots: Vec<_> = (0..12)
        .map(|i| alloc.new_slot(format!("string_{}", i)))
        .collect();

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(*slot.get(), format!("string_{}", i));
    }
}

// =============================================================================
// Key Access
// =============================================================================

#[test]
fn miri_get_unchecked() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.leak();

    let value = unsafe { alloc.get_by_key_unchecked(key) };
    assert_eq!(*value, 42);
}

#[test]
fn miri_get_unchecked_mut() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.leak();

    unsafe {
        *alloc.get_by_key_unchecked_mut(key) = 100;
    }

    let value = unsafe { alloc.get_by_key_unchecked(key) };
    assert_eq!(*value, 100);
}

#[test]
fn miri_get_unchecked_string() {
    let alloc: Allocator<String> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot("hello".to_string());
    let key = slot.leak();

    let value = unsafe { alloc.get_by_key_unchecked(key) };
    assert_eq!(*value, "hello");
}

#[test]
fn miri_get_valid_key() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.leak();

    let value = unsafe { alloc.get_by_key(key) };
    assert_eq!(value, Some(&42));
}

#[test]
fn miri_get_invalid_key() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.key();
    drop(slot); // Slot is now vacant

    let value = unsafe { alloc.get_by_key(key) };
    assert_eq!(value, None);
}

#[test]
fn miri_get_string() {
    let alloc: Allocator<String> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot("hello world".to_string());
    let key = slot.leak();

    let value = unsafe { alloc.get_by_key(key) };
    assert_eq!(value.map(|s| s.as_str()), Some("hello world"));
}

#[test]
fn miri_get_mut_valid_key() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.leak();

    unsafe {
        if let Some(v) = alloc.get_by_key_mut(key) {
            *v = 100;
        }
    }

    let value = unsafe { alloc.get_by_key(key) };
    assert_eq!(value, Some(&100));
}

#[test]
fn miri_get_mut_invalid_key() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.key();
    drop(slot);

    let value = unsafe { alloc.get_by_key_mut(key) };
    assert!(value.is_none());
}

#[test]
fn miri_get_mut_string() {
    let alloc: Allocator<String> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot("hello".to_string());
    let key = slot.leak();

    unsafe {
        if let Some(s) = alloc.get_by_key_mut(key) {
            s.push_str(" world");
        }
    }

    let value = unsafe { alloc.get_by_key(key) };
    assert_eq!(value.map(|s| s.as_str()), Some("hello world"));
}

#[test]
fn miri_try_remove_by_key_valid() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.leak();

    assert_eq!(alloc.len(), 1);

    let value = unsafe { alloc.try_remove_by_key(key) };
    assert_eq!(value, Some(42));
    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_try_remove_by_key_invalid() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.key();
    drop(slot);

    let value = unsafe { alloc.try_remove_by_key(key) };
    assert_eq!(value, None);
}

#[test]
fn miri_try_remove_by_key_string() {
    let alloc: Allocator<String> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot("hello".to_string());
    let key = slot.leak();

    let value = unsafe { alloc.try_remove_by_key(key) };
    assert_eq!(value, Some("hello".to_string()));
    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_remove_by_key_basic() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let key = slot.leak();

    let value = unsafe { alloc.remove_by_key(key) };
    assert_eq!(value, 42);
    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_remove_by_key_string() {
    let alloc: Allocator<String> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot("hello world".to_string());
    let key = slot.leak();

    let value = unsafe { alloc.remove_by_key(key) };
    assert_eq!(value, "hello world");
    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_remove_by_key_drops_correctly() {
    reset_drop_count();

    let alloc: Allocator<DropTracker> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(DropTracker(1));
    let key = slot.leak();

    assert_eq!(get_drop_count(), 0);

    let value = unsafe { alloc.remove_by_key(key) };
    assert_eq!(get_drop_count(), 0); // Not dropped yet - we own it

    drop(value);
    assert_eq!(get_drop_count(), 1); // Now dropped
}

#[test]
fn miri_remove_by_key_slot_reused() {
    let alloc: Allocator<String> = Allocator::builder().bounded(1).build();

    let slot = alloc.new_slot("first".to_string());
    let key = slot.leak();

    // Remove via key
    let value = unsafe { alloc.remove_by_key(key) };
    assert_eq!(value, "first");

    // Slot should be reusable
    let slot2 = alloc.new_slot("second".to_string());
    assert_eq!(*slot2, "second");
    assert_eq!(slot2.key().index(), key.index()); // Same slot reused
}

#[test]
fn miri_remove_by_key_vec() {
    let alloc: Allocator<Vec<u64>> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(vec![1, 2, 3, 4, 5]);
    let key = slot.leak();

    let value = unsafe { alloc.remove_by_key(key) };
    assert_eq!(value, vec![1, 2, 3, 4, 5]);
    assert_eq!(alloc.len(), 0);
}

// =============================================================================
// Pointer Methods
// =============================================================================

#[test]
fn miri_as_ptr() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let slot = alloc.new_slot(42);
    let ptr = slot.as_ptr();
    assert_eq!(unsafe { *ptr }, 42);
}

#[test]
fn miri_as_mut_ptr() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(4).build();

    let mut slot = alloc.new_slot(42);
    let ptr = slot.as_mut_ptr();
    unsafe { *ptr = 100 };
    assert_eq!(*slot, 100);
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn miri_capacity_one() {
    let alloc: Allocator<u64> = Allocator::builder().bounded(1).build();

    let slot = alloc.new_slot(42);
    assert!(alloc.try_new_slot(100).is_none());
    drop(slot);

    let slot2 = alloc.new_slot(100);
    assert_eq!(*slot2, 100);
}

#[test]
fn miri_zst() {
    let alloc: Allocator<ZeroSized> = Allocator::builder().bounded(10).build();

    let slot = alloc.new_slot(ZeroSized);
    assert_eq!(*slot, ZeroSized);
    drop(slot);
    assert_eq!(alloc.len(), 0);
}

#[test]
fn miri_large_struct() {
    let alloc: Allocator<Large> = Allocator::builder().bounded(4).build();

    let mut data = [0u64; 128];
    for (i, d) in data.iter_mut().enumerate() {
        *d = i as u64;
    }

    let slot = alloc.new_slot(Large { data });
    assert_eq!(slot.data[0], 0);
    assert_eq!(slot.data[127], 127);
}
