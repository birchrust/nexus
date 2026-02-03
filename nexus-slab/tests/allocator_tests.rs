//! Comprehensive tests for the Slab API.
//!
//! This test suite covers:
//! - Basic operations (bounded and unbounded)
//! - Drop semantics and tracking
//! - Stress tests and freelist integrity
//! - Edge cases and boundary conditions
//! - Complex types (String, Vec, ZST, large)
//! - Key validity and contains_key behavior

use nexus_slab::bounded::{Slab as BoundedSlab, Slot as BoundedSlot};
use nexus_slab::unbounded::Slab as UnboundedSlab;
use nexus_slab::Key;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

// =============================================================================
// Helper Types
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
    let slab = BoundedSlab::<u64>::new(16);

    assert_eq!(slab.len(), 0);
    assert!(slab.is_empty());
    assert_eq!(slab.capacity(), 16);

    {
        let slot = slab.new_slot(42);
        assert_eq!(*slot, 42);
        assert_eq!(slab.len(), 1);
        assert!(!slab.is_empty());
    }

    assert_eq!(slab.len(), 0);
}

#[test]
fn bounded_leak_and_key_access() {
    let slab = BoundedSlab::<u64>::new(16);

    let slot = slab.new_slot(42);
    let key = slot.leak();

    assert!(slab.contains_key(key));
    assert_eq!(slab.len(), 1);

    let value = unsafe { slab.get_by_key(key) }.unwrap();
    assert_eq!(*value, 42);

    // Modify via mutable access
    unsafe {
        *slab.get_by_key_mut(key).unwrap() = 100;
    }
    let value = unsafe { slab.get_by_key(key) }.unwrap();
    assert_eq!(*value, 100);
}

#[test]
fn bounded_fill_to_capacity() {
    let slab = BoundedSlab::<u64>::new(8);

    let slots: Vec<_> = (0..8).map(|i| slab.new_slot(i)).collect();

    assert_eq!(slab.len(), 8);
    assert_eq!(slab.capacity(), 8);
    assert!(slab.try_new_slot(100).is_err());

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, i as u64);
    }
}

#[test]
fn bounded_capacity_one() {
    let slab = BoundedSlab::<u64>::new(1);

    assert_eq!(slab.capacity(), 1);

    let slot = slab.new_slot(42);
    assert!(slab.try_new_slot(100).is_err());

    let key = slot.key();
    assert_eq!(key.index(), 0);
    assert!(slab.contains_key(key));

    drop(slot);
    assert!(!slab.contains_key(key));

    let slot2 = slab.new_slot(100);
    assert_eq!(*slot2, 100);
    assert_eq!(slot2.key().index(), 0); // Same slot reused
}

// =============================================================================
// Basic Operations - Unbounded
// =============================================================================

#[test]
fn unbounded_basic_insert_drop() {
    let slab = UnboundedSlab::<u64>::new(8);

    assert_eq!(slab.len(), 0);

    {
        let slot = slab.new_slot(100);
        assert_eq!(*slot, 100);
        assert_eq!(slab.len(), 1);
    }

    assert_eq!(slab.len(), 0);
}

#[test]
fn unbounded_grows_automatically() {
    let slab = UnboundedSlab::<u64>::new(4);

    let initial_cap = slab.capacity();

    // Insert more than initial chunk
    let slots: Vec<_> = (0..20).map(|i| slab.new_slot(i)).collect();

    assert_eq!(slab.len(), 20);
    assert!(slab.capacity() >= 20);
    assert!(slab.capacity() > initial_cap);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, i as u64);
    }
}

#[test]
fn unbounded_cross_chunk_access() {
    let slab = UnboundedSlab::<u64>::new(4);

    // Fill multiple chunks
    let mut slots: Vec<_> = (0..16).map(|i| slab.new_slot(i)).collect();

    // Verify all values accessible
    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, i as u64);
    }

    // Leak some from different chunks (remove in reverse order to maintain indices)
    let key12 = slots.remove(12).leak(); // Chunk 3
    let key8 = slots.remove(8).leak(); // Chunk 2
    let key4 = slots.remove(4).leak(); // Chunk 1
    let key0 = slots.remove(0).leak(); // Chunk 0

    assert!(slab.contains_key(key0));
    assert!(slab.contains_key(key4));
    assert!(slab.contains_key(key8));
    assert!(slab.contains_key(key12));

    assert_eq!(*unsafe { slab.get_by_key(key0) }.unwrap(), 0);
    assert_eq!(*unsafe { slab.get_by_key(key4) }.unwrap(), 4);
    assert_eq!(*unsafe { slab.get_by_key(key8) }.unwrap(), 8);
    assert_eq!(*unsafe { slab.get_by_key(key12) }.unwrap(), 12);

    // Drop the rest
    drop(slots);
}

// =============================================================================
// Slot Operations
// =============================================================================

#[test]
fn slot_deref() {
    let slab = BoundedSlab::<u64>::new(4);

    let mut slot = slab.new_slot(42);
    assert_eq!(*slot, 42);

    *slot = 100;
    assert_eq!(*slot, 100);
}

#[test]
fn slot_into_inner() {
    let slab = BoundedSlab::<String>::new(4);

    let slot = slab.new_slot("hello".to_string());
    let key = slot.key();
    let value = slot.into_inner();

    assert_eq!(value, "hello");
    assert_eq!(slab.len(), 0);
    assert!(!slab.contains_key(key));
}

#[test]
fn slot_replace() {
    let slab = BoundedSlab::<u64>::new(4);

    let mut slot = slab.new_slot(1);
    let old = slot.replace(2);
    assert_eq!(old, 1);
    assert_eq!(*slot, 2);

    let old2 = slot.replace(3);
    assert_eq!(old2, 2);
    assert_eq!(*slot, 3);
}

#[test]
fn slot_key_matches_after_leak() {
    let slab = BoundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    let key_before = slot.key();
    let key_after = slot.leak();

    assert_eq!(key_before, key_after);
    assert!(slab.contains_key(key_after));
}

#[test]
fn slot_debug_format() {
    let slab = BoundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    let debug = format!("{:?}", slot);
    assert!(debug.contains("Slot"));
    assert!(debug.contains("key"));
}

#[test]
fn slot_size_is_16_bytes() {
    assert_eq!(std::mem::size_of::<BoundedSlot<u64>>(), 16);
    assert_eq!(std::mem::size_of::<BoundedSlot<String>>(), 16);
    assert_eq!(std::mem::size_of::<BoundedSlot<[u8; 1024]>>(), 16);
}

// =============================================================================
// Multiple Slots and Slabs
// =============================================================================

#[test]
fn multiple_slots_same_slab() {
    let slab = BoundedSlab::<u64>::new(10);

    let slot1 = slab.new_slot(1);
    let slot2 = slab.new_slot(2);
    let slot3 = slab.new_slot(3);

    assert_eq!(slab.len(), 3);
    assert_eq!(*slot1, 1);
    assert_eq!(*slot2, 2);
    assert_eq!(*slot3, 3);

    // Keys should be different
    assert_ne!(slot1.key(), slot2.key());
    assert_ne!(slot2.key(), slot3.key());
    assert_ne!(slot1.key(), slot3.key());

    drop(slot2);
    assert_eq!(slab.len(), 2);

    // Insert again - should reuse slot2's slot
    let slot4 = slab.new_slot(4);
    assert_eq!(slab.len(), 3);
    assert_eq!(*slot4, 4);
}

#[test]
fn multiple_slabs_independent() {
    let slab_a = BoundedSlab::<u64>::new(4);
    let slab_b = BoundedSlab::<u64>::new(4);

    let slot_a = slab_a.new_slot(1);
    let slot_b = slab_b.new_slot(2);

    assert_eq!(slab_a.len(), 1);
    assert_eq!(slab_b.len(), 1);

    assert_eq!(*slot_a, 1);
    assert_eq!(*slot_b, 2);

    drop(slot_a);
    assert_eq!(slab_a.len(), 0);
    assert_eq!(slab_b.len(), 1);
}

// =============================================================================
// Key Validity and contains_key
// =============================================================================

#[test]
fn key_none_never_valid() {
    let slab = BoundedSlab::<u64>::new(4);

    let _slot = slab.new_slot(42);
    assert!(!slab.contains_key(Key::NONE));
}

#[test]
fn key_out_of_bounds_not_valid() {
    let slab = BoundedSlab::<u64>::new(4);

    let _slot = slab.new_slot(42);

    // Index beyond capacity
    assert!(!slab.contains_key(Key::new(100)));
    assert!(!slab.contains_key(Key::new(4)));
    assert!(!slab.contains_key(Key::new(1000)));
}

#[test]
fn key_invalid_after_drop() {
    let slab = BoundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    let key = slot.key();
    assert!(slab.contains_key(key));

    drop(slot);
    assert!(!slab.contains_key(key));
}

#[test]
fn key_invalid_after_into_inner() {
    let slab = BoundedSlab::<String>::new(4);

    let slot = slab.new_slot("hello".to_string());
    let key = slot.key();
    assert!(slab.contains_key(key));

    let _value = slot.into_inner();
    assert!(!slab.contains_key(key));
}

#[test]
fn get_returns_some_for_valid_key() {
    let slab = BoundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    let key = slot.leak();

    let value = unsafe { slab.get_by_key(key) };
    assert_eq!(value, Some(&42));
}

#[test]
fn get_returns_none_for_invalid_key() {
    let slab = BoundedSlab::<u64>::new(4);

    let _slot = slab.new_slot(42);

    // Invalid key
    let value = unsafe { slab.get_by_key(Key::new(99)) };
    assert_eq!(value, None);

    // Key::NONE
    let value = unsafe { slab.get_by_key(Key::NONE) };
    assert_eq!(value, None);
}

#[test]
fn get_mut_returns_some_and_allows_mutation() {
    let slab = BoundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    let key = slot.leak();

    if let Some(value) = unsafe { slab.get_by_key_mut(key) } {
        *value = 100;
    }

    let value = unsafe { slab.get_by_key(key) };
    assert_eq!(value, Some(&100));
}

#[test]
fn get_mut_returns_none_for_invalid_key() {
    let slab = BoundedSlab::<u64>::new(4);

    let _slot = slab.new_slot(42);

    let value = unsafe { slab.get_by_key_mut(Key::new(99)) };
    assert!(value.is_none());
}

#[test]
fn try_remove_by_key_returns_some_for_valid() {
    let slab = BoundedSlab::<String>::new(4);

    let slot = slab.new_slot("hello".to_string());
    let key = slot.leak();

    let value = unsafe { slab.remove_by_key(key) };
    assert_eq!(value, Some("hello".to_string()));
    assert!(!slab.contains_key(key));
}

#[test]
fn try_remove_by_key_returns_none_for_invalid() {
    let slab = BoundedSlab::<u64>::new(4);

    let _slot = slab.new_slot(42);

    let value = unsafe { slab.remove_by_key(Key::new(99)) };
    assert_eq!(value, None);

    let value = unsafe { slab.remove_by_key(Key::NONE) };
    assert_eq!(value, None);
}

#[test]
fn remove_by_key_returns_value() {
    let slab = BoundedSlab::<String>::new(4);

    let slot = slab.new_slot("hello".to_string());
    let key = slot.leak();

    assert!(slab.contains_key(key));
    assert_eq!(slab.len(), 1);

    let value = unsafe { slab.remove_by_key(key) }.unwrap();
    assert_eq!(value, "hello");
    assert!(!slab.contains_key(key));
    assert_eq!(slab.len(), 0);
}

#[test]
fn remove_by_key_drops_correctly() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    let slot = slab.new_slot(DropTracker(42));
    let key = slot.leak();

    assert_eq!(get_drop_count(), 0);

    let value = unsafe { slab.remove_by_key(key) }.unwrap();
    // Value returned, not dropped yet
    assert_eq!(get_drop_count(), 0);

    drop(value);
    // Now dropped
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn remove_by_key_slot_reused() {
    let slab = BoundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    let key = slot.leak();
    let index = key.index();

    unsafe { slab.remove_by_key(key) };

    // Insert again - should reuse the same slot
    let new_slot = slab.new_slot(100);
    assert_eq!(new_slot.key().index(), index);
}

#[test]
fn key_still_valid_after_leak() {
    let slab = BoundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    let key = slot.leak();
    assert!(slab.contains_key(key));
    // Leaked - stays valid
}

#[test]
fn contains_key_vacant_slot() {
    let slab = BoundedSlab::<u64>::new(4);

    // Slot 0 exists but is vacant
    assert!(!slab.contains_key(Key::new(0)));

    let slot = slab.new_slot(42);
    assert!(slab.contains_key(Key::new(0)));

    drop(slot);
    // Back to vacant
    assert!(!slab.contains_key(Key::new(0)));
}

// =============================================================================
// Panic Tests
// =============================================================================

#[test]
#[should_panic(expected = "slab full")]
fn panic_insert_when_full() {
    let slab = BoundedSlab::<u64>::new(2);

    let _s1 = slab.new_slot(1);
    let _s2 = slab.new_slot(2);
    let _ = slab.new_slot(3); // Should panic
}

#[test]
#[should_panic(expected = "capacity must be non-zero")]
fn panic_zero_capacity() {
    let _ = BoundedSlab::<u64>::new(0);
}

// =============================================================================
// Drop Semantics
// =============================================================================

#[test]
fn drop_called_on_slot_drop() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    {
        let _slot = slab.new_slot(DropTracker(1));
        assert_eq!(get_drop_count(), 0);
    }

    assert_eq!(get_drop_count(), 1);
}

#[test]
fn drop_called_multiple() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(10);

    {
        let _s1 = slab.new_slot(DropTracker(1));
        let _s2 = slab.new_slot(DropTracker(2));
        let _s3 = slab.new_slot(DropTracker(3));
        assert_eq!(get_drop_count(), 0);
    }

    assert_eq!(get_drop_count(), 3);
}

#[test]
fn drop_called_on_into_inner() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    let slot = slab.new_slot(DropTracker(1));
    let value = slot.into_inner();
    assert_eq!(get_drop_count(), 0); // Not dropped yet - returned

    drop(value);
    assert_eq!(get_drop_count(), 1); // Now dropped
}

#[test]
fn drop_not_called_after_leak() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    {
        let slot = slab.new_slot(DropTracker(1));
        let _key = slot.leak();
        // Slot forgotten, value stays alive
    }

    assert_eq!(get_drop_count(), 0); // Leaked, not dropped
}

#[test]
fn drop_called_on_replace() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    let mut slot = slab.new_slot(DropTracker(1));
    assert_eq!(get_drop_count(), 0);

    let old = slot.replace(DropTracker(2));
    assert_eq!(get_drop_count(), 0); // Old returned, not dropped yet

    drop(old);
    assert_eq!(get_drop_count(), 1); // Old dropped

    drop(slot);
    assert_eq!(get_drop_count(), 2); // New value dropped
}

#[test]
fn drop_order_all_dropped() {
    DROP_ORDER.store(0, Ordering::SeqCst);

    let slab = BoundedSlab::<OrderedDrop>::new(4);

    {
        let _s1 = slab.new_slot(OrderedDrop { id: 1 });
        let _s2 = slab.new_slot(OrderedDrop { id: 2 });
        let _s3 = slab.new_slot(OrderedDrop { id: 3 });
        // Drops in reverse order: s3, s2, s1
    }

    assert_eq!(DROP_ORDER.load(Ordering::SeqCst), 3);
}

// =============================================================================
// Stress Tests and Freelist Integrity
// =============================================================================

#[test]
fn stress_fill_drain_cycle() {
    let slab = BoundedSlab::<u64>::new(100);

    for cycle in 0..10 {
        // Fill
        let slots: Vec<_> = (0..100)
            .map(|i| slab.new_slot(i + cycle * 100))
            .collect();
        assert_eq!(slab.len(), 100);

        // Verify values
        for (i, slot) in slots.iter().enumerate() {
            assert_eq!(**slot, (i + cycle as usize * 100) as u64);
        }

        // Drain
        drop(slots);
        assert_eq!(slab.len(), 0);
    }
}

#[test]
fn stress_interleaved_insert_remove() {
    let slab = BoundedSlab::<u64>::new(50);

    let mut slots = Vec::new();

    for i in 0..1000 {
        if i % 2 == 0 || slots.is_empty() {
            // Insert
            if slab.len() < 50 {
                slots.push(slab.new_slot(i));
            }
        } else {
            // Remove (drop last)
            slots.pop();
        }
    }

    assert_eq!(slab.len(), slots.len());
}

#[test]
fn stress_slot_reuse() {
    let slab = BoundedSlab::<u64>::new(1);

    for i in 0..1000 {
        let slot = slab.new_slot(i);
        assert_eq!(*slot, i);
        assert_eq!(slot.key().index(), 0); // Always same slot
    }
}

#[test]
fn stress_unbounded_growth() {
    let slab = UnboundedSlab::<u64>::new(16);

    let slots: Vec<_> = (0..1000).map(|i| slab.new_slot(i)).collect();

    assert_eq!(slab.len(), 1000);
    assert!(slab.capacity() >= 1000);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, i as u64);
    }
}

#[test]
fn stress_unbounded_churn() {
    let slab = UnboundedSlab::<u64>::new(8);

    let mut slots = Vec::new();

    for i in 0..500 {
        // Add some
        for j in 0..5 {
            slots.push(slab.new_slot((i * 5 + j) as u64));
        }

        // Remove some
        for _ in 0..3 {
            if !slots.is_empty() {
                let _ = slots.swap_remove(i % slots.len().max(1));
            }
        }
    }

    assert_eq!(slab.len(), slots.len());
}

#[test]
fn freelist_lifo_order() {
    let slab = BoundedSlab::<u64>::new(4);

    // Insert 4 items
    let s0 = slab.new_slot(0);
    let s1 = slab.new_slot(1);
    let s2 = slab.new_slot(2);
    let s3 = slab.new_slot(3);

    let _k0 = s0.key();
    let k1 = s1.key();
    let _k2 = s2.key();
    let k3 = s3.key();

    // Drop in order: s1, s3
    drop(s1);
    drop(s3);

    // Freelist should have: s3 -> s1 (LIFO)
    // Next insert should get s3's slot
    let new1 = slab.new_slot(100);
    assert_eq!(new1.key(), k3);

    let new2 = slab.new_slot(101);
    assert_eq!(new2.key(), k1);
}

// =============================================================================
// Complex Types
// =============================================================================

#[test]
fn type_string() {
    let slab = BoundedSlab::<String>::new(10);

    let slot = slab.new_slot("hello world".to_string());
    assert_eq!(*slot, "hello world");

    let key = slot.key();
    let value = slot.into_inner();
    assert_eq!(value, "hello world");
    assert!(!slab.contains_key(key));
}

#[test]
fn type_vec() {
    let slab = BoundedSlab::<Vec<u64>>::new(10);

    let slot = slab.new_slot(vec![1, 2, 3, 4, 5]);
    assert_eq!(slot.len(), 5);
    assert_eq!(slot[2], 3);
}

#[test]
fn type_box() {
    let slab = BoundedSlab::<Box<u64>>::new(10);

    let slot = slab.new_slot(Box::new(42));
    assert_eq!(**slot, 42);
}

#[test]
fn type_rc() {
    let slab = BoundedSlab::<Rc<u64>>::new(10);

    let rc = Rc::new(42);
    let slot = slab.new_slot(rc.clone());

    assert_eq!(Rc::strong_count(&rc), 2);
    drop(slot);
    assert_eq!(Rc::strong_count(&rc), 1);
}

#[test]
fn type_option() {
    let slab = BoundedSlab::<Option<String>>::new(10);

    let slot1 = slab.new_slot(Some("hello".to_string()));
    let slot2 = slab.new_slot(None);

    assert_eq!(*slot1, Some("hello".to_string()));
    assert_eq!(*slot2, None);
}

#[test]
fn type_tuple() {
    let slab = BoundedSlab::<(u64, String, bool)>::new(10);

    let slot = slab.new_slot((42, "hello".to_string(), true));
    assert_eq!(slot.0, 42);
    assert_eq!(slot.1, "hello");
    assert!(slot.2);
}

#[test]
fn type_large_struct() {
    let slab = BoundedSlab::<LargeStruct>::new(10);

    let mut data = [0u64; 128];
    for (i, d) in data.iter_mut().enumerate() {
        *d = i as u64;
    }

    let slot = slab.new_slot(LargeStruct { data });

    for (i, &d) in slot.data.iter().enumerate() {
        assert_eq!(d, i as u64);
    }
}

#[test]
fn type_zst() {
    let slab = BoundedSlab::<ZeroSized>::new(100);

    assert_eq!(std::mem::size_of::<ZeroSized>(), 0);

    let slot = slab.new_slot(ZeroSized);
    assert_eq!(*slot, ZeroSized);

    // Can still track it
    assert_eq!(slab.len(), 1);
    drop(slot);
    assert_eq!(slab.len(), 0);
}

#[test]
fn type_unit() {
    let slab = BoundedSlab::<()>::new(10);

    let slot = slab.new_slot(());
    assert_eq!(*slot, ());
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn large_capacity() {
    let slab = BoundedSlab::<u64>::new(100_000);

    assert_eq!(slab.capacity(), 100_000);

    let slots: Vec<_> = (0..1000).map(|i| slab.new_slot(i)).collect();
    assert_eq!(slab.len(), 1000);

    drop(slots);
    assert_eq!(slab.len(), 0);
}

#[test]
fn unbounded_default_chunk_capacity() {
    let slab = UnboundedSlab::<u64>::new(4096);

    // First insert should trigger chunk allocation
    let _slot = slab.new_slot(42);
    assert!(slab.capacity() >= 1);
}

#[test]
fn key_roundtrip_raw() {
    let slab = BoundedSlab::<u64>::new(10);

    let slot = slab.new_slot(42);
    let key = slot.key();
    let raw = key.into_raw();
    let restored = Key::from_raw(raw);

    assert_eq!(key, restored);
    assert!(slab.contains_key(restored));
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
fn slab_is_copy() {
    let slab = BoundedSlab::<u64>::new(10);
    let slab2 = slab; // Copy
    let slab3 = slab; // Copy again

    // All refer to same underlying storage
    let slot = slab.new_slot(42);
    assert_eq!(slab.len(), 1);
    assert_eq!(slab2.len(), 1);
    assert_eq!(slab3.len(), 1);

    drop(slot);
    assert_eq!(slab.len(), 0);
    assert_eq!(slab2.len(), 0);
}

#[test]
fn slab_debug_format() {
    let slab = BoundedSlab::<u64>::new(10);
    let debug = format!("{:?}", slab);
    assert!(debug.contains("Slab"));
    assert!(debug.contains("len"));
    assert!(debug.contains("capacity"));
}
