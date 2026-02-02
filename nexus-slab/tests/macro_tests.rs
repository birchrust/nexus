//! Comprehensive tests for the create_allocator! macro.
//!
//! This test suite covers:
//! - Basic operations (bounded and unbounded)
//! - Panic conditions
//! - Drop semantics and tracking
//! - Stress tests and freelist integrity
//! - Edge cases and boundary conditions
//! - Complex types (String, Vec, ZST, large)
//! - Key validity and contains_key behavior

use nexus_slab::{Key, create_allocator};
use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

// =============================================================================
// Helper Types (at module level for macro visibility)
// Must use crate:: prefix when passing to create_allocator!
// =============================================================================

thread_local! {
    static DROP_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug)]
pub struct DropTracker(pub u64);

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

pub struct LargeStruct {
    pub data: [u64; 128],
}

pub struct OrderedDrop {
    pub id: usize,
}

static DROP_ORDER: AtomicUsize = AtomicUsize::new(0);

impl Drop for OrderedDrop {
    fn drop(&mut self) {
        DROP_ORDER.fetch_add(1, Ordering::SeqCst);
    }
}

// =============================================================================
// Basic Operations - Bounded
// =============================================================================

#[test]
fn bounded_basic_insert_drop() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(16).build();

    assert!(test_alloc::is_initialized());
    assert_eq!(test_alloc::len(), 0);
    assert!(test_alloc::is_empty());
    assert_eq!(test_alloc::capacity(), 16);

    {
        let slot = test_alloc::insert(42);
        assert_eq!(*slot.get(), 42);
        assert_eq!(test_alloc::len(), 1);
        assert!(!test_alloc::is_empty());
    }

    assert_eq!(test_alloc::len(), 0);
    assert!(test_alloc::shutdown().is_ok());
    assert!(!test_alloc::is_initialized());
}

#[test]
fn bounded_leak_and_key_access() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(16).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    assert!(test_alloc::contains_key(key));
    assert_eq!(test_alloc::len(), 1);

    let value = unsafe { test_alloc::get_unchecked(key) };
    assert_eq!(*value, 42);

    // Modify via mutable access
    unsafe {
        *test_alloc::get_unchecked_mut(key) = 100;
    }
    let value = unsafe { test_alloc::get_unchecked(key) };
    assert_eq!(*value, 100);
}

#[test]
fn bounded_fill_to_capacity() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(8).build();

    let slots: Vec<_> = (0..8).map(|i| test_alloc::insert(i)).collect();

    assert_eq!(test_alloc::len(), 8);
    assert_eq!(test_alloc::capacity(), 8);
    assert!(test_alloc::try_insert(100).is_none());

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(*slot.get(), i as u64);
    }
}

#[test]
fn bounded_capacity_one() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(1).build();

    assert_eq!(test_alloc::capacity(), 1);

    let slot = test_alloc::insert(42);
    assert!(test_alloc::try_insert(100).is_none());

    let key = slot.key();
    assert_eq!(key.index(), 0);
    assert!(test_alloc::contains_key(key));

    drop(slot);
    assert!(!test_alloc::contains_key(key));

    let slot2 = test_alloc::insert(100);
    assert_eq!(*slot2, 100);
    assert_eq!(slot2.key().index(), 0); // Same slot reused
}

// =============================================================================
// Basic Operations - Unbounded
// =============================================================================

#[test]
fn unbounded_basic_insert_drop() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(8).build();

    assert!(test_alloc::is_initialized());
    assert_eq!(test_alloc::len(), 0);

    {
        let slot = test_alloc::insert(100);
        assert_eq!(*slot.get(), 100);
        assert_eq!(test_alloc::len(), 1);
    }

    assert_eq!(test_alloc::len(), 0);
    assert!(test_alloc::shutdown().is_ok());
}

#[test]
fn unbounded_grows_automatically() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(4).build();

    // Chunk capacity rounds to power of 2, so 4 stays 4
    let initial_cap = test_alloc::capacity();

    // Insert more than initial chunk
    let slots: Vec<_> = (0..20).map(|i| test_alloc::insert(i)).collect();

    assert_eq!(test_alloc::len(), 20);
    assert!(test_alloc::capacity() >= 20);
    assert!(test_alloc::capacity() > initial_cap);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(*slot.get(), i as u64);
    }
}

#[test]
fn unbounded_with_prealloc() {
    create_allocator!(test_alloc, u64);
    test_alloc::init()
        .unbounded()
        .chunk_capacity(16)
        .capacity(100)
        .build();

    assert!(test_alloc::capacity() >= 100);
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn unbounded_cross_chunk_access() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(4).build();

    // Fill multiple chunks
    let mut slots: Vec<_> = (0..16).map(|i| test_alloc::insert(i)).collect();

    // Verify all values accessible
    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(*slot.get(), i as u64);
        assert!(slot.is_valid());
    }

    // Leak some from different chunks (remove in reverse order to maintain indices)
    let key12 = slots.remove(12).leak(); // Chunk 3
    let key8 = slots.remove(8).leak();   // Chunk 2
    let key4 = slots.remove(4).leak();   // Chunk 1
    let key0 = slots.remove(0).leak();   // Chunk 0

    assert!(test_alloc::contains_key(key0));
    assert!(test_alloc::contains_key(key4));
    assert!(test_alloc::contains_key(key8));
    assert!(test_alloc::contains_key(key12));

    assert_eq!(*unsafe { test_alloc::get_unchecked(key0) }, 0);
    assert_eq!(*unsafe { test_alloc::get_unchecked(key4) }, 4);
    assert_eq!(*unsafe { test_alloc::get_unchecked(key8) }, 8);
    assert_eq!(*unsafe { test_alloc::get_unchecked(key12) }, 12);

    // Drop the rest
    drop(slots);
}

// =============================================================================
// Slot Operations
// =============================================================================

#[test]
fn slot_deref() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(42);
    assert_eq!(*slot, 42);

    *slot = 100;
    assert_eq!(*slot, 100);
}

#[test]
fn slot_into_inner() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let key = slot.key();
    let value = slot.into_inner();

    assert_eq!(value, "hello");
    assert_eq!(test_alloc::len(), 0);
    assert!(!test_alloc::contains_key(key));
}

#[test]
fn slot_replace() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(1);
    let old = slot.replace(2);
    assert_eq!(old, 1);
    assert_eq!(*slot, 2);

    let old2 = slot.replace(3);
    assert_eq!(old2, 2);
    assert_eq!(*slot, 3);
}

#[test]
fn slot_pointer_methods() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(42);

    let ptr = slot.as_ptr();
    assert_eq!(unsafe { *ptr }, 42);

    let mut_ptr = slot.as_mut_ptr();
    unsafe { *mut_ptr = 100 };
    assert_eq!(*slot, 100);
}

#[test]
fn slot_key_matches_after_leak() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key_before = slot.key();
    let key_after = slot.leak();

    assert_eq!(key_before, key_after);
    assert!(test_alloc::contains_key(key_after));
}

#[test]
fn slot_is_valid() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    assert!(slot.is_valid());
}

#[test]
fn slot_debug_format() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let debug = format!("{:?}", slot);
    assert!(debug.contains("Slot"));
    assert!(debug.contains("key"));
}

#[test]
fn slot_size_is_8_bytes() {
    create_allocator!(test_alloc, u64);
    assert_eq!(std::mem::size_of::<test_alloc::Slot>(), 8);

    create_allocator!(test_alloc2, String);
    assert_eq!(std::mem::size_of::<test_alloc2::Slot>(), 8);

    create_allocator!(test_alloc3, [u8; 1024]);
    assert_eq!(std::mem::size_of::<test_alloc3::Slot>(), 8);
}

// =============================================================================
// Multiple Slots and Allocators
// =============================================================================

#[test]
fn multiple_slots_same_allocator() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(10).build();

    let slot1 = test_alloc::insert(1);
    let slot2 = test_alloc::insert(2);
    let slot3 = test_alloc::insert(3);

    assert_eq!(test_alloc::len(), 3);
    assert_eq!(*slot1, 1);
    assert_eq!(*slot2, 2);
    assert_eq!(*slot3, 3);

    // Keys should be different
    assert_ne!(slot1.key(), slot2.key());
    assert_ne!(slot2.key(), slot3.key());
    assert_ne!(slot1.key(), slot3.key());

    drop(slot2);
    assert_eq!(test_alloc::len(), 2);

    // Insert again - should reuse slot2's slot
    let slot4 = test_alloc::insert(4);
    assert_eq!(test_alloc::len(), 3);
    assert_eq!(*slot4, 4);
}

#[test]
fn multiple_allocators_independent() {
    create_allocator!(alloc_a, u64);
    create_allocator!(alloc_b, u64);

    alloc_a::init().bounded(4).build();
    alloc_b::init().bounded(4).build();

    let slot_a = alloc_a::insert(1);
    let slot_b = alloc_b::insert(2);

    assert_eq!(alloc_a::len(), 1);
    assert_eq!(alloc_b::len(), 1);

    assert_eq!(*slot_a, 1);
    assert_eq!(*slot_b, 2);

    drop(slot_a);
    assert_eq!(alloc_a::len(), 0);
    assert_eq!(alloc_b::len(), 1);
}

// =============================================================================
// Uninitialized State
// =============================================================================

#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "allocator not initialized"))]
fn try_insert_when_not_initialized() {
    create_allocator!(test_alloc, u64);
    // In debug: panics with "allocator not initialized"
    // In release: null pointer dereference (test won't run in release anyway for should_panic)
    let _ = test_alloc::try_insert(42);
}

#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "allocator not initialized"))]
fn contains_key_when_not_initialized() {
    create_allocator!(test_alloc, u64);
    // In debug: panics with "allocator not initialized"
    let _ = test_alloc::contains_key(Key::new(0));
}

#[test]
fn shutdown_when_not_initialized() {
    create_allocator!(test_alloc, u64);
    assert!(test_alloc::shutdown().is_ok());
}

#[test]
fn len_capacity_when_not_initialized() {
    create_allocator!(test_alloc, u64);
    assert_eq!(test_alloc::len(), 0);
    assert_eq!(test_alloc::capacity(), 0);
    assert!(test_alloc::is_empty());
}

// =============================================================================
// Shutdown Behavior
// =============================================================================

#[test]
fn shutdown_fails_with_outstanding_slots() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let _slot = test_alloc::insert(42);

    let result = test_alloc::shutdown();
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().0, 1);
}

#[test]
fn shutdown_fails_with_multiple_outstanding() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(10).build();

    let _s1 = test_alloc::insert(1);
    let _s2 = test_alloc::insert(2);
    let _s3 = test_alloc::insert(3);

    let result = test_alloc::shutdown();
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().0, 3);
}

#[test]
fn shutdown_succeeds_after_all_dropped() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    {
        let _slot1 = test_alloc::insert(1);
        let _slot2 = test_alloc::insert(2);
        assert_eq!(test_alloc::len(), 2);
    }

    assert_eq!(test_alloc::len(), 0);
    assert!(test_alloc::shutdown().is_ok());
}

#[test]
fn shutdown_error_display() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let _slot = test_alloc::insert(42);
    let err = test_alloc::shutdown().unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("1 slots still in use"));
}

// =============================================================================
// Key Validity and contains_key
// =============================================================================

#[test]
fn key_none_never_valid() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let _slot = test_alloc::insert(42);
    assert!(!test_alloc::contains_key(Key::NONE));
}

#[test]
fn key_out_of_bounds_not_valid() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let _slot = test_alloc::insert(42);

    // Index beyond capacity
    assert!(!test_alloc::contains_key(Key::new(100)));
    assert!(!test_alloc::contains_key(Key::new(4)));
    assert!(!test_alloc::contains_key(Key::new(1000)));
}

#[test]
fn key_invalid_after_drop() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.key();
    assert!(test_alloc::contains_key(key));

    drop(slot);
    assert!(!test_alloc::contains_key(key));
}

#[test]
fn key_invalid_after_into_inner() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let key = slot.key();
    assert!(test_alloc::contains_key(key));

    let _value = slot.into_inner();
    assert!(!test_alloc::contains_key(key));
}

#[test]
fn get_returns_some_for_valid_key() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    let value = unsafe { test_alloc::get(key) };
    assert_eq!(value, Some(&42));
}

#[test]
fn get_returns_none_for_invalid_key() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let _slot = test_alloc::insert(42);

    // Invalid key
    let value = unsafe { test_alloc::get(Key::new(99)) };
    assert_eq!(value, None);

    // Key::NONE
    let value = unsafe { test_alloc::get(Key::NONE) };
    assert_eq!(value, None);
}

#[test]
fn get_mut_returns_some_and_allows_mutation() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();

    if let Some(value) = unsafe { test_alloc::get_mut(key) } {
        *value = 100;
    }

    let value = unsafe { test_alloc::get(key) };
    assert_eq!(value, Some(&100));
}

#[test]
fn get_mut_returns_none_for_invalid_key() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let _slot = test_alloc::insert(42);

    let value = unsafe { test_alloc::get_mut(Key::new(99)) };
    assert!(value.is_none());
}

#[test]
fn try_remove_by_key_returns_some_for_valid() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let key = slot.leak();

    let value = unsafe { test_alloc::try_remove_by_key(key) };
    assert_eq!(value, Some("hello".to_string()));
    assert!(!test_alloc::contains_key(key));
}

#[test]
fn try_remove_by_key_returns_none_for_invalid() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let _slot = test_alloc::insert(42);

    let value = unsafe { test_alloc::try_remove_by_key(Key::new(99)) };
    assert_eq!(value, None);

    let value = unsafe { test_alloc::try_remove_by_key(Key::NONE) };
    assert_eq!(value, None);
}

#[test]
fn remove_by_key_returns_value() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let key = slot.leak();

    assert!(test_alloc::contains_key(key));
    assert_eq!(test_alloc::len(), 1);

    let value = unsafe { test_alloc::remove_by_key(key) };
    assert_eq!(value, "hello");
    assert!(!test_alloc::contains_key(key));
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn remove_by_key_drops_correctly() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(crate::DropTracker(42));
    let key = slot.leak();

    assert_eq!(get_drop_count(), 0);

    let value = unsafe { test_alloc::remove_by_key(key) };
    // Value returned, not dropped yet
    assert_eq!(get_drop_count(), 0);

    drop(value);
    // Now dropped
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn remove_by_key_slot_reused() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();
    let index = key.index();

    unsafe { test_alloc::remove_by_key(key) };

    // Insert again - should reuse the same slot
    let new_slot = test_alloc::insert(100);
    assert_eq!(new_slot.key().index(), index);
}

#[test]
fn key_still_valid_after_leak() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(42);
    let key = slot.leak();
    assert!(test_alloc::contains_key(key));
    // Leaked - stays valid
}

#[test]
fn contains_key_vacant_slot() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    // Slot 0 exists but is vacant
    assert!(!test_alloc::contains_key(Key::new(0)));

    let slot = test_alloc::insert(42);
    assert!(test_alloc::contains_key(Key::new(0)));

    drop(slot);
    // Back to vacant
    assert!(!test_alloc::contains_key(Key::new(0)));
}

// =============================================================================
// Panic Tests
// =============================================================================

#[test]
#[should_panic(expected = "allocator not initialized")]
fn panic_insert_not_initialized() {
    create_allocator!(test_alloc, u64);
    let _ = test_alloc::insert(42);
}

#[test]
#[should_panic(expected = "allocator full or not initialized")]
fn panic_insert_when_full() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(2).build();

    let _s1 = test_alloc::insert(1);
    let _s2 = test_alloc::insert(2);
    let _ = test_alloc::insert(3); // Should panic
}

#[test]
#[should_panic(expected = "allocator already initialized")]
fn panic_double_init_bounded() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();
    test_alloc::init().bounded(4).build(); // Should panic
}

#[test]
#[should_panic(expected = "allocator already initialized")]
fn panic_double_init_unbounded() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().build();
    test_alloc::init().unbounded().build(); // Should panic
}

#[test]
#[should_panic(expected = "capacity must be non-zero")]
fn panic_zero_capacity() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(0).build();
}

#[test]
#[should_panic(expected = "chunk_capacity must be non-zero")]
fn panic_zero_chunk_capacity() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(0).build();
}

// =============================================================================
// Drop Semantics
// =============================================================================

#[test]
fn drop_called_on_slot_drop() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    {
        let _slot = test_alloc::insert(crate::DropTracker(1));
        assert_eq!(get_drop_count(), 0);
    }

    assert_eq!(get_drop_count(), 1);
}

#[test]
fn drop_called_multiple() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(10).build();

    {
        let _s1 = test_alloc::insert(crate::DropTracker(1));
        let _s2 = test_alloc::insert(crate::DropTracker(2));
        let _s3 = test_alloc::insert(crate::DropTracker(3));
        assert_eq!(get_drop_count(), 0);
    }

    assert_eq!(get_drop_count(), 3);
}

#[test]
fn drop_called_on_into_inner() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(crate::DropTracker(1));
    let value = slot.into_inner();
    assert_eq!(get_drop_count(), 0); // Not dropped yet - returned

    drop(value);
    assert_eq!(get_drop_count(), 1); // Now dropped
}

#[test]
fn drop_not_called_after_leak() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    {
        let slot = test_alloc::insert(crate::DropTracker(1));
        let _key = slot.leak();
        // Slot forgotten, value stays alive
    }

    assert_eq!(get_drop_count(), 0); // Leaked, not dropped
}

#[test]
fn drop_called_on_replace() {
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    let mut slot = test_alloc::insert(crate::DropTracker(1));
    assert_eq!(get_drop_count(), 0);

    let old = slot.replace(crate::DropTracker(2));
    assert_eq!(get_drop_count(), 0); // Old returned, not dropped yet

    drop(old);
    assert_eq!(get_drop_count(), 1); // Old dropped

    drop(slot);
    assert_eq!(get_drop_count(), 2); // New value dropped
}

#[test]
fn drop_order_all_dropped() {
    // Verify all items are dropped (uses module-level OrderedDrop)
    DROP_ORDER.store(0, Ordering::SeqCst);

    create_allocator!(test_alloc, crate::OrderedDrop);
    test_alloc::init().bounded(4).build();

    {
        let _s1 = test_alloc::insert(crate::OrderedDrop { id: 1 });
        let _s2 = test_alloc::insert(crate::OrderedDrop { id: 2 });
        let _s3 = test_alloc::insert(crate::OrderedDrop { id: 3 });
        // Drops in reverse order: s3, s2, s1
    }

    assert_eq!(DROP_ORDER.load(Ordering::SeqCst), 3);
}

// =============================================================================
// Stress Tests and Freelist Integrity
// =============================================================================

#[test]
fn stress_fill_drain_cycle() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(100).build();

    for cycle in 0..10 {
        // Fill
        let slots: Vec<_> = (0..100).map(|i| test_alloc::insert(i + cycle * 100)).collect();
        assert_eq!(test_alloc::len(), 100);

        // Verify values
        for (i, slot) in slots.iter().enumerate() {
            assert_eq!(*slot.get(), (i + cycle as usize * 100) as u64);
        }

        // Drain
        drop(slots);
        assert_eq!(test_alloc::len(), 0);
    }
}

#[test]
fn stress_interleaved_insert_remove() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(50).build();

    let mut slots = Vec::new();

    for i in 0..1000 {
        if i % 2 == 0 || slots.is_empty() {
            // Insert
            if test_alloc::len() < 50 {
                slots.push(test_alloc::insert(i));
            }
        } else {
            // Remove (drop last)
            slots.pop();
        }
    }

    assert_eq!(test_alloc::len(), slots.len());
}

#[test]
fn stress_slot_reuse() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(1).build();

    for i in 0..1000 {
        let slot = test_alloc::insert(i);
        assert_eq!(*slot, i);
        assert_eq!(slot.key().index(), 0); // Always same slot
    }
}

#[test]
fn stress_unbounded_growth() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(16).build();

    let slots: Vec<_> = (0..1000).map(|i| test_alloc::insert(i)).collect();

    assert_eq!(test_alloc::len(), 1000);
    assert!(test_alloc::capacity() >= 1000);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(*slot.get(), i as u64);
    }
}

#[test]
fn stress_unbounded_churn() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(8).build();

    let mut slots = Vec::new();

    for i in 0..500 {
        // Add some
        for j in 0..5 {
            slots.push(test_alloc::insert((i * 5 + j) as u64));
        }

        // Remove some
        for _ in 0..3 {
            if !slots.is_empty() {
                slots.swap_remove(i % slots.len().max(1));
            }
        }
    }

    // Verify remaining are valid
    for slot in &slots {
        assert!(slot.is_valid());
    }

    assert_eq!(test_alloc::len(), slots.len());
}

#[test]
fn freelist_lifo_order() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    // Insert 4 items
    let s0 = test_alloc::insert(0);
    let s1 = test_alloc::insert(1);
    let s2 = test_alloc::insert(2);
    let s3 = test_alloc::insert(3);

    let k0 = s0.key();
    let k1 = s1.key();
    let k2 = s2.key();
    let k3 = s3.key();

    // Drop in order: s1, s3
    drop(s1);
    drop(s3);

    // Freelist should have: s3 -> s1 (LIFO)
    // Next insert should get s3's slot
    let new1 = test_alloc::insert(100);
    assert_eq!(new1.key(), k3);

    let new2 = test_alloc::insert(101);
    assert_eq!(new2.key(), k1);
}

// =============================================================================
// Complex Types
// =============================================================================

#[test]
fn type_string() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(10).build();

    let slot = test_alloc::insert("hello world".to_string());
    assert_eq!(*slot, "hello world");

    let key = slot.key();
    let value = slot.into_inner();
    assert_eq!(value, "hello world");
    assert!(!test_alloc::contains_key(key));
}

#[test]
fn type_vec() {
    create_allocator!(test_alloc, Vec<u64>);
    test_alloc::init().bounded(10).build();

    let slot = test_alloc::insert(vec![1, 2, 3, 4, 5]);
    assert_eq!(slot.len(), 5);
    assert_eq!(slot[2], 3);
}

#[test]
fn type_box() {
    create_allocator!(test_alloc, Box<u64>);
    test_alloc::init().bounded(10).build();

    let slot = test_alloc::insert(Box::new(42));
    assert_eq!(**slot, 42);
}

#[test]
fn type_rc() {
    create_allocator!(test_alloc, std::rc::Rc<u64>);
    test_alloc::init().bounded(10).build();

    let rc = Rc::new(42);
    let slot = test_alloc::insert(rc.clone());

    assert_eq!(Rc::strong_count(&rc), 2);
    drop(slot);
    assert_eq!(Rc::strong_count(&rc), 1);
}

#[test]
fn type_option() {
    create_allocator!(test_alloc, Option<String>);
    test_alloc::init().bounded(10).build();

    let slot1 = test_alloc::insert(Some("hello".to_string()));
    let slot2 = test_alloc::insert(None);

    assert_eq!(*slot1, Some("hello".to_string()));
    assert_eq!(*slot2, None);
}

#[test]
fn type_tuple() {
    create_allocator!(test_alloc, (u64, String, bool));
    test_alloc::init().bounded(10).build();

    let slot = test_alloc::insert((42, "hello".to_string(), true));
    assert_eq!(slot.0, 42);
    assert_eq!(slot.1, "hello");
    assert!(slot.2);
}

#[test]
fn type_large_struct() {
    create_allocator!(test_alloc, crate::LargeStruct);
    test_alloc::init().bounded(10).build();

    let mut data = [0u64; 128];
    for (i, d) in data.iter_mut().enumerate() {
        *d = i as u64;
    }

    let slot = test_alloc::insert(crate::LargeStruct { data });

    for (i, &d) in slot.data.iter().enumerate() {
        assert_eq!(d, i as u64);
    }
}

#[test]
fn type_zst() {
    create_allocator!(test_alloc, crate::ZeroSized);
    test_alloc::init().bounded(100).build();

    assert_eq!(std::mem::size_of::<crate::ZeroSized>(), 0);

    let slot = test_alloc::insert(crate::ZeroSized);
    assert_eq!(*slot, crate::ZeroSized);

    // Can still track it
    assert_eq!(test_alloc::len(), 1);
    drop(slot);
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn type_unit() {
    create_allocator!(test_alloc, ());
    test_alloc::init().bounded(10).build();

    let slot = test_alloc::insert(());
    assert_eq!(*slot, ());
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn large_capacity() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(100_000).build();

    assert_eq!(test_alloc::capacity(), 100_000);

    let slots: Vec<_> = (0..1000).map(|i| test_alloc::insert(i)).collect();
    assert_eq!(test_alloc::len(), 1000);

    drop(slots);
    assert_eq!(test_alloc::len(), 0);
}

#[test]
fn unbounded_default_chunk_capacity() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().build();

    // Default chunk capacity is 4096
    // First insert should trigger allocation
    let _slot = test_alloc::insert(42);
    assert!(test_alloc::capacity() >= 1);
}

#[test]
fn key_roundtrip_raw() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(10).build();

    let slot = test_alloc::insert(42);
    let key = slot.key();
    let raw = key.into_raw();
    let restored = Key::from_raw(raw);

    assert_eq!(key, restored);
    assert!(test_alloc::contains_key(restored));
}

#[test]
fn key_debug_format() {
    let key = Key::new(42);
    let debug = format!("{:?}", key);
    assert_eq!(debug, "Key(42)");

    let none_debug = format!("{:?}", Key::NONE);
    assert_eq!(none_debug, "Key::NONE");
}

#[test]
fn concurrent_init_protection() {
    // Can't truly test threading since allocator is !Send,
    // but verify the protection mechanism works
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(4).build();

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        test_alloc::init().bounded(4).build();
    }));

    assert!(result.is_err());
}

// =============================================================================
// Reinit After Shutdown
// =============================================================================

#[test]
fn reinit_after_shutdown_bounded() {
    create_allocator!(test_alloc, u64);

    // First init
    test_alloc::init().bounded(4).build();
    let slot = test_alloc::insert(1);
    drop(slot);
    assert!(test_alloc::shutdown().is_ok());
    assert!(!test_alloc::is_initialized());

    // Second init
    test_alloc::init().bounded(8).build();
    assert!(test_alloc::is_initialized());
    assert_eq!(test_alloc::capacity(), 8);

    let slot = test_alloc::insert(2);
    assert_eq!(*slot, 2);
}

#[test]
fn reinit_after_shutdown_unbounded() {
    create_allocator!(test_alloc, u64);

    // First init
    test_alloc::init().unbounded().chunk_capacity(4).build();
    let slot = test_alloc::insert(1);
    drop(slot);
    assert!(test_alloc::shutdown().is_ok());

    // Second init
    test_alloc::init().unbounded().chunk_capacity(16).build();
    assert!(test_alloc::is_initialized());

    let slot = test_alloc::insert(2);
    assert_eq!(*slot, 2);
}

// =============================================================================
// Memory Safety (basic tests - full Miri coverage in miri_tests.rs)
// =============================================================================

#[test]
fn no_use_after_free() {
    create_allocator!(test_alloc, String);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert("hello".to_string());
    let key = slot.key();
    drop(slot);

    // Key should not be valid
    assert!(!test_alloc::contains_key(key));

    // Reusing slot should work
    let new_slot = test_alloc::insert("world".to_string());
    assert_eq!(*new_slot, "world");
}

#[test]
fn no_double_free() {
    // This test ensures that into_inner doesn't double-free
    reset_drop_count();

    create_allocator!(test_alloc, crate::DropTracker);
    test_alloc::init().bounded(4).build();

    let slot = test_alloc::insert(crate::DropTracker(1));
    let value = slot.into_inner();
    // Value is returned, not dropped
    assert_eq!(get_drop_count(), 0);

    drop(value);
    // Dropped exactly once
    assert_eq!(get_drop_count(), 1);

    // Slab should be empty
    assert_eq!(test_alloc::len(), 0);
}
