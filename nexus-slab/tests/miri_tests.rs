//! Miri-specific tests for memory safety verification.
//!
//! Run with: `cargo +nightly miri test --test miri_tests -- -Zmiri-ignore-leaks`
//!
//! The `-Zmiri-ignore-leaks` flag is required because:
//! - VTables are intentionally leaked (Box::leak for stable addresses)
//! - Leaked slots (via `slot.leak()`) are intentionally not freed
//!
//! These tests verify:
//! - No use-after-free
//! - No double-free
//! - No uninitialized memory access
//! - No invalid pointer arithmetic
//! - Correct drop ordering

use nexus_slab::create_allocator;
use std::cell::Cell;

// =============================================================================
// Helper Types (at module level for macro visibility)
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
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(8).build();

    let slot = test_alloc::insert(42);
    assert_eq!(*slot, 42);
    drop(slot);

    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_unbounded_basic() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(4).build();

    let slot = test_alloc::insert(42);
    assert_eq!(*slot, 42);
    drop(slot);

    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_multiple_inserts() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(8).build();

    let s1 = test_alloc::insert(1);
    let s2 = test_alloc::insert(2);
    let s3 = test_alloc::insert(3);

    assert_eq!(*s1, 1);
    assert_eq!(*s2, 2);
    assert_eq!(*s3, 3);
}

#[test]
fn miri_slot_deref_mut() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(42);
    *slot = 100;
    assert_eq!(*slot, 100);
}

#[test]
fn miri_slot_replace() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(1);
    let old = slot.replace(2);
    assert_eq!(old, 1);
    assert_eq!(*slot, 2);
}

#[test]
fn miri_slot_into_inner() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let value = slot.into_inner();
    assert_eq!(value, 42);
    assert_eq!(test_alloc::len(), 0);
}

// =============================================================================
// Drop Safety
// =============================================================================

#[test]
fn miri_drop_on_slot_drop() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    {
        let _slot = test_alloc::insert(crate::DropTracker(1));
    }

    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_into_inner() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(crate::DropTracker(1));
    let value = slot.into_inner();
    assert_eq!(get_drop_count(), 0);

    drop(value);
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_replace() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(crate::DropTracker(1));
    let old = slot.replace(DropTracker(2));
    drop(old);
    assert_eq!(get_drop_count(), 1);

    drop(slot);
    assert_eq!(get_drop_count(), 2);
}

#[test]
fn miri_no_drop_after_leak() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(crate::DropTracker(1));
    let _key = slot.leak();

    assert_eq!(get_drop_count(), 0);
}

// =============================================================================
// Heap-Allocated Types
// =============================================================================

#[test]
fn miri_string_insert_drop() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello world".to_string());
    assert_eq!(*slot, "hello world");
    drop(slot);

    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_vec_insert_drop() {
    create_allocator!(test_alloc, Vec<u64>);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(vec![1, 2, 3, 4, 5]);
    assert_eq!(slot.len(), 5);
    drop(slot);

    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_box_insert_drop() {
    create_allocator!(test_alloc, Box<[u8; 1024]>);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(Box::new([0u8; 1024]));
    assert_eq!(slot.len(), 1024);
    drop(slot);

    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_string_into_inner() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let value = slot.into_inner();
    assert_eq!(value, "hello");
}

#[test]
fn miri_vec_replace() {
    create_allocator!(test_alloc, Vec<u64>);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(vec![1, 2, 3]);
    let old = slot.replace(vec![4, 5, 6, 7]);

    assert_eq!(old, vec![1, 2, 3]);
    assert_eq!(*slot, vec![4, 5, 6, 7]);
}

// =============================================================================
// Slot Reuse
// =============================================================================

#[test]
fn miri_slot_reuse_bounded() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(2).build();

    // Fill
    let s1 = test_alloc::insert("one".to_string());
    let s2 = test_alloc::insert("two".to_string());

    let k1 = s1.key();
    let k2 = s2.key();

    // Free one
    drop(s1);

    // Reuse
    let s3 = test_alloc::insert("three".to_string());
    assert_eq!(*s3, "three");
    assert_eq!(s3.key(), k1); // Reused slot 1

    // Free other
    drop(s2);

    // Reuse again
    let s4 = test_alloc::insert("four".to_string());
    assert_eq!(*s4, "four");
    assert_eq!(s4.key(), k2); // Reused slot 2
}

#[test]
fn miri_slot_reuse_single() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(1).build();

    for i in 0..10 {
        let slot = test_alloc::insert(format!("value_{}", i));
        assert_eq!(*slot, format!("value_{}", i));
        assert_eq!(slot.key().index(), 0);
    }
}

// =============================================================================
// Unbounded Growth
// =============================================================================

#[test]
fn miri_unbounded_growth() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(4).build();

    // Fill multiple chunks
    let slots: Vec<_> = (0..12).map(|i| test_alloc::insert(i)).collect();

    assert_eq!(test_alloc::len(), 12);
    assert!(test_alloc::capacity() >= 12);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(*slot.get(), i as u64);
    }
}

#[test]
fn miri_unbounded_string_growth() {
    create_allocator!(test_alloc, String);
    test_alloc::init().unbounded().chunk_capacity(4).build();

    let slots: Vec<_> = (0..12)
        .map(|i| test_alloc::insert(format!("string_{}", i)))
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
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    let value = unsafe { test_alloc::get_unchecked(key) };
    assert_eq!(*value, 42);
}

#[test]
fn miri_get_unchecked_mut() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    unsafe {
        *test_alloc::get_unchecked_mut(key) = 100;
    }

    let value = unsafe { test_alloc::get_unchecked(key) };
    assert_eq!(*value, 100);
}

#[test]
fn miri_get_unchecked_string() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let key = slot.leak();

    let value = unsafe { test_alloc::get_unchecked(key) };
    assert_eq!(*value, "hello");
}

#[test]
fn miri_get_valid_key() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    let value = unsafe { test_alloc::get(key) };
    assert_eq!(value, Some(&42));
}

#[test]
fn miri_get_invalid_key() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.key();
    drop(slot); // Slot is now vacant

    let value = unsafe { test_alloc::get(key) };
    assert_eq!(value, None);
}

#[test]
fn miri_get_string() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello world".to_string());
    let key = slot.leak();

    let value = unsafe { test_alloc::get(key) };
    assert_eq!(value.map(|s| s.as_str()), Some("hello world"));
}

#[test]
fn miri_get_mut_valid_key() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    unsafe {
        if let Some(v) = test_alloc::get_mut(key) {
            *v = 100;
        }
    }

    let value = unsafe { test_alloc::get(key) };
    assert_eq!(value, Some(&100));
}

#[test]
fn miri_get_mut_invalid_key() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.key();
    drop(slot);

    let value = unsafe { test_alloc::get_mut(key) };
    assert!(value.is_none());
}

#[test]
fn miri_get_mut_string() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let key = slot.leak();

    unsafe {
        if let Some(s) = test_alloc::get_mut(key) {
            s.push_str(" world");
        }
    }

    let value = unsafe { test_alloc::get(key) };
    assert_eq!(value.map(|s| s.as_str()), Some("hello world"));
}

#[test]
fn miri_try_remove_by_key_valid() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    assert_eq!(test_alloc::len(), 1);

    let value = unsafe { test_alloc::try_remove_by_key(key) };
    assert_eq!(value, Some(42));
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_try_remove_by_key_invalid() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.key();
    drop(slot);

    let value = unsafe { test_alloc::try_remove_by_key(key) };
    assert_eq!(value, None);
}

#[test]
fn miri_try_remove_by_key_string() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let key = slot.leak();

    let value = unsafe { test_alloc::try_remove_by_key(key) };
    assert_eq!(value, Some("hello".to_string()));
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_remove_by_key_basic() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    let value = unsafe { test_alloc::remove_by_key(key) };
    assert_eq!(value, 42);
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_remove_by_key_string() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello world".to_string());
    let key = slot.leak();

    let value = unsafe { test_alloc::remove_by_key(key) };
    assert_eq!(value, "hello world");
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_remove_by_key_drops_correctly() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(crate::DropTracker(1));
    let key = slot.leak();

    assert_eq!(get_drop_count(), 0);

    let value = unsafe { test_alloc::remove_by_key(key) };
    assert_eq!(get_drop_count(), 0); // Not dropped yet - we own it

    drop(value);
    assert_eq!(get_drop_count(), 1); // Now dropped
}

#[test]
fn miri_remove_by_key_slot_reused() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(1).build();

    let slot = test_alloc::insert("first".to_string());
    let key = slot.leak();

    // Remove via key
    let value = unsafe { test_alloc::remove_by_key(key) };
    assert_eq!(value, "first");

    // Slot should be reusable
    let slot2 = test_alloc::insert("second".to_string());
    assert_eq!(*slot2, "second");
    assert_eq!(slot2.key().index(), key.index()); // Same slot reused
}

#[test]
fn miri_remove_by_key_vec() {
    create_allocator!(test_alloc, Vec<u64>);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(vec![1, 2, 3, 4, 5]);
    let key = slot.leak();

    let value = unsafe { test_alloc::remove_by_key(key) };
    assert_eq!(value, vec![1, 2, 3, 4, 5]);
    assert_eq!(test_alloc::len(), 0);
}

// =============================================================================
// Pointer Methods
// =============================================================================

#[test]
fn miri_as_ptr() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let ptr = slot.as_ptr();
    assert_eq!(unsafe { *ptr }, 42);
}

#[test]
fn miri_as_mut_ptr() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(42);
    let ptr = slot.as_mut_ptr();
    unsafe { *ptr = 100 };
    assert_eq!(*slot, 100);
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn miri_capacity_one() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(1).build();

    let slot = test_alloc::insert(42);
    assert!(test_alloc::try_insert(100).is_none());
    drop(slot);

    let slot2 = test_alloc::insert(100);
    assert_eq!(*slot2, 100);
}

#[test]
fn miri_zst() {
    create_allocator!(test_alloc, crate::ZeroSized);
    test_alloc::init().bounded(10).build();

    let slot = test_alloc::insert(crate::ZeroSized);
    assert_eq!(*slot, crate::ZeroSized);
    drop(slot);
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn miri_large_struct() {
    create_allocator!(test_alloc, crate::Large);
    test_alloc::init().bounded(4).build();

    let mut data = [0u64; 128];
    for (i, d) in data.iter_mut().enumerate() {
        *d = i as u64;
    }

    let slot = test_alloc::insert(crate::Large { data });
    assert_eq!(slot.data[0], 0);
    assert_eq!(slot.data[127], 127);
}

// =============================================================================
// Shutdown
// =============================================================================

#[test]
fn miri_shutdown_empty() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    assert!(test_alloc::shutdown().is_ok());
    assert!(!test_alloc::is_initialized());
}

#[test]
fn miri_shutdown_after_drops() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    {
        let _s1 = test_alloc::insert("one".to_string());
        let _s2 = test_alloc::insert("two".to_string());
    }

    assert!(test_alloc::shutdown().is_ok());
}

#[test]
fn miri_reinit_after_shutdown() {
    create_allocator!(test_alloc, u64);

    test_alloc::init().bounded(4).build();
    let slot = test_alloc::insert(1);
    drop(slot);
    assert!(test_alloc::shutdown().is_ok());

    test_alloc::init().bounded(8).build();
    let slot = test_alloc::insert(2);
    assert_eq!(*slot, 2);
}
