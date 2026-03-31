//! Miri-specific tests for memory safety verification.
//!
//! Run with: `MIRIFLAGS="-Zmiri-ignore-leaks" cargo +nightly miri test --test miri_tests`
//!
//! The `-Zmiri-ignore-leaks` flag is required because:
//! - Slabs are intentionally leaked (Box::leak for stable addresses)
//! - Leaked slots (via `slot.leak()`) are intentionally not freed
//!
//! These tests verify:
//! - No use-after-free
//! - No double-free
//! - No uninitialized memory access
//! - No invalid pointer arithmetic
//! - Correct drop ordering

use nexus_slab::bounded::Slab as BoundedSlab;
use nexus_slab::unbounded::Slab as UnboundedSlab;
use std::cell::Cell;

// =============================================================================
// Helper Types
// =============================================================================

thread_local! {
    static DROP_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug)]
pub struct DropTracker(#[allow(dead_code)] u64);

impl Drop for DropTracker {
    fn drop(&mut self) {
        DROP_COUNT.with(|c| c.set(c.get() + 1));
    }
}

fn reset_drop_count() {
    DROP_COUNT.with(|c| c.set(0));
}

fn get_drop_count() -> usize {
    DROP_COUNT.with(Cell::get)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(8) };

    let slot = slab.alloc(42);
    assert_eq!(*slot, 42);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_unbounded_basic() {
    let slab = unsafe { UnboundedSlab::<u64>::with_chunk_capacity(4) };

    let slot = slab.alloc(42);
    assert_eq!(*slot, 42);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_multiple_inserts() {
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(8) };

    let s1 = slab.alloc(1);
    let s2 = slab.alloc(2);
    let s3 = slab.alloc(3);

    assert_eq!(*s1, 1);
    assert_eq!(*s2, 2);
    assert_eq!(*s3, 3);

    slab.free(s1);
    slab.free(s2);
    slab.free(s3);
}

#[test]
fn miri_slot_deref_mut() {
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(4) };

    let mut slot = slab.alloc(42);
    *slot = 100;
    assert_eq!(*slot, 100);

    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_slot_replace() {
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(4) };

    let mut slot = slab.alloc(1);
    let old = std::mem::replace(&mut *slot, 2);
    assert_eq!(old, 1);
    assert_eq!(*slot, 2);

    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_slot_into_inner() {
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(4) };

    let slot = slab.alloc(42);
    // SAFETY: slot was allocated from this slab
    let value = slab.take(slot);
    assert_eq!(value, 42);
}

// =============================================================================
// Drop Safety
// =============================================================================

#[test]
fn miri_drop_on_slot_drop() {
    reset_drop_count();

    let slab = unsafe { BoundedSlab::<DropTracker>::with_capacity(4) };

    {
        let slot = slab.alloc(DropTracker(1));
        // SAFETY: slot was allocated from this slab
        slab.free(slot);
    }

    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_into_inner() {
    reset_drop_count();

    let slab = unsafe { BoundedSlab::<DropTracker>::with_capacity(4) };

    let slot = slab.alloc(DropTracker(1));
    // SAFETY: slot was allocated from this slab
    let value = slab.take(slot);
    assert_eq!(get_drop_count(), 0);

    drop(value);
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_replace() {
    reset_drop_count();

    let slab = unsafe { BoundedSlab::<DropTracker>::with_capacity(4) };

    let mut slot = slab.alloc(DropTracker(1));
    let old = std::mem::replace(&mut *slot, DropTracker(2));
    drop(old);
    assert_eq!(get_drop_count(), 1);

    // SAFETY: slot was allocated from this slab
    slab.free(slot);
    assert_eq!(get_drop_count(), 2);
}

#[test]
fn miri_no_drop_after_leak() {
    reset_drop_count();

    let slab = unsafe { BoundedSlab::<DropTracker>::with_capacity(4) };

    let slot = slab.alloc(DropTracker(1));
    // Intentionally leak — disarm debug Drop via into_raw()
    let _ = slot.into_raw();

    assert_eq!(get_drop_count(), 0);
}

// =============================================================================
// Heap-Allocated Types
// =============================================================================

#[test]
fn miri_string_insert_drop() {
    let slab = unsafe { BoundedSlab::<String>::with_capacity(4) };

    let slot = slab.alloc("hello world".to_string());
    assert_eq!(*slot, "hello world");
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_vec_insert_drop() {
    // SAFETY: slab outlives all slots
    let slab = unsafe { BoundedSlab::<Vec<u64>>::with_capacity(4) };

    let slot = slab.alloc(vec![1, 2, 3, 4, 5]);
    assert_eq!(slot.len(), 5);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_box_insert_drop() {
    // SAFETY: slab outlives all slots
    let slab = unsafe { BoundedSlab::<Box<[u8; 1024]>>::with_capacity(4) };

    let slot = slab.alloc(Box::new([0u8; 1024]));
    assert_eq!(slot.len(), 1024);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_string_into_inner() {
    let slab = unsafe { BoundedSlab::<String>::with_capacity(4) };

    let slot = slab.alloc("hello".to_string());
    // SAFETY: slot was allocated from this slab
    let value = slab.take(slot);
    assert_eq!(value, "hello");
}

#[test]
fn miri_vec_replace() {
    // SAFETY: slab outlives all slots
    let slab = unsafe { BoundedSlab::<Vec<u64>>::with_capacity(4) };

    let mut slot = slab.alloc(vec![1, 2, 3]);
    let old = std::mem::replace(&mut *slot, vec![4, 5, 6, 7]);

    assert_eq!(old, vec![1, 2, 3]);
    assert_eq!(*slot, vec![4, 5, 6, 7]);

    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

// =============================================================================
// Slot Reuse
// =============================================================================

#[test]
fn miri_slot_reuse_bounded() {
    let slab = unsafe { BoundedSlab::<String>::with_capacity(2) };

    // Fill
    let s1 = slab.alloc("one".to_string());
    let s2 = slab.alloc("two".to_string());

    let p1 = s1.as_ptr();
    let p2 = s2.as_ptr();

    // Dealloc one
    // SAFETY: slot was allocated from this slab
    slab.free(s1);

    // Reuse
    let s3 = slab.alloc("three".to_string());
    assert_eq!(*s3, "three");
    assert_eq!(s3.as_ptr(), p1); // Reused slot 1

    // Dealloc other
    // SAFETY: slot was allocated from this slab
    slab.free(s2);

    // Reuse again
    let s4 = slab.alloc("four".to_string());
    assert_eq!(*s4, "four");
    assert_eq!(s4.as_ptr(), p2); // Reused slot 2

    // Clean up
    slab.free(s3);
    slab.free(s4);
}

#[test]
fn miri_slot_reuse_single() {
    let slab = unsafe { BoundedSlab::<String>::with_capacity(1) };

    let mut last_ptr = std::ptr::null_mut();
    for i in 0..10 {
        let slot = slab.alloc(format!("value_{}", i));
        assert_eq!(*slot, format!("value_{}", i));
        // After first iteration, should always reuse same slot
        if i > 0 {
            assert_eq!(slot.as_ptr(), last_ptr);
        }
        last_ptr = slot.as_ptr();
        // SAFETY: slot was allocated from this slab
        slab.free(slot);
    }
}

// =============================================================================
// Unbounded Growth
// =============================================================================

#[test]
fn miri_unbounded_growth() {
    let slab = unsafe { UnboundedSlab::<u64>::with_chunk_capacity(4) };

    // Fill multiple chunks
    let slots: Vec<_> = (0..12).map(|i| slab.alloc(i)).collect();

    assert!(slab.capacity() >= 12);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, i as u64);
    }

    // Clean up
    for slot in slots {
        // SAFETY: slot was allocated from this slab
        slab.free(slot);
    }
}

#[test]
fn miri_unbounded_string_growth() {
    let slab = unsafe { UnboundedSlab::<String>::with_chunk_capacity(4) };

    let slots: Vec<_> = (0..12)
        .map(|i| slab.alloc(format!("string_{}", i)))
        .collect();

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, format!("string_{}", i));
    }

    // Clean up
    for slot in slots {
        // SAFETY: slot was allocated from this slab
        slab.free(slot);
    }
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn miri_capacity_one() {
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(1) };

    let slot = slab.alloc(42);
    assert!(slab.try_alloc(100).is_err());
    // SAFETY: slot was allocated from this slab
    slab.free(slot);

    let slot2 = slab.alloc(100);
    assert_eq!(*slot2, 100);
    // SAFETY: slot was allocated from this slab
    slab.free(slot2);
}

#[test]
fn miri_zst() {
    let slab = unsafe { BoundedSlab::<ZeroSized>::with_capacity(10) };

    let slot = slab.alloc(ZeroSized);
    assert_eq!(*slot, ZeroSized);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_large_struct() {
    let slab = unsafe { BoundedSlab::<Large>::with_capacity(4) };

    let mut data = [0u64; 128];
    for (i, d) in data.iter_mut().enumerate() {
        *d = i as u64;
    }

    let slot = slab.alloc(Large { data });
    assert_eq!(slot.data[0], 0);
    assert_eq!(slot.data[127], 127);

    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

// =============================================================================
// Claim Abandonment (H5)
// =============================================================================

#[test]
fn miri_bounded_claim_abandon() {
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(4) };

    // Claim and abandon — should return slot to freelist
    {
        let claim = slab.claim().unwrap();
        drop(claim);
    }

    // Slab should still be at full capacity
    let slots: Vec<_> = (0..4).map(|i| slab.alloc(i)).collect();
    for slot in slots {
        // SAFETY: slot was allocated from this slab
        slab.free(slot);
    }
}

#[test]
fn miri_bounded_claim_abandon_capacity_one() {
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(1) };

    // Claim and abandon
    {
        let claim = slab.claim().unwrap();
        drop(claim);
    }

    // Should be able to allocate again
    let slot = slab.alloc(42);
    assert_eq!(*slot, 42);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_unbounded_claim_abandon() {
    let slab = unsafe { UnboundedSlab::<u64>::with_chunk_capacity(4) };

    // Allocate and free to ensure chunk exists
    let slot = slab.alloc(0);
    slab.free(slot);

    // Claim and abandon
    {
        let claim = slab.claim();
        drop(claim);
    }

    // Should still be able to allocate
    let slot = slab.alloc(99);
    assert_eq!(*slot, 99);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_unbounded_claim_abandon_full_chunk() {
    let slab = unsafe { UnboundedSlab::<u64>::with_chunk_capacity(2) };

    // Fill first chunk
    let s1 = slab.alloc(1);
    let s2 = slab.alloc(2);

    // Claim from second chunk, then abandon
    {
        let claim = slab.claim();
        drop(claim);
    }

    // Should still be able to allocate from that chunk
    let s3 = slab.alloc(3);
    assert_eq!(*s3, 3);

    slab.free(s1);
    slab.free(s2);
    slab.free(s3);
}

// =============================================================================
// Claim::write (L8)
// =============================================================================

#[test]
fn miri_bounded_claim_write() {
    let slab = unsafe { BoundedSlab::<u64>::with_capacity(4) };

    let claim = slab.claim().unwrap();
    let slot = claim.write(42);
    assert_eq!(*slot, 42);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_bounded_claim_write_string() {
    let slab = unsafe { BoundedSlab::<String>::with_capacity(4) };

    let claim = slab.claim().unwrap();
    let slot = claim.write("hello world".to_string());
    assert_eq!(*slot, "hello world");
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}

#[test]
fn miri_bounded_claim_write_drop_type() {
    reset_drop_count();

    let slab = unsafe { BoundedSlab::<DropTracker>::with_capacity(4) };

    let claim = slab.claim().unwrap();
    let slot = claim.write(DropTracker(1));
    assert_eq!(get_drop_count(), 0);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_unbounded_claim_write() {
    let slab = unsafe { UnboundedSlab::<u64>::with_chunk_capacity(4) };

    let claim = slab.claim();
    let slot = claim.write(99);
    assert_eq!(*slot, 99);
    // SAFETY: slot was allocated from this slab
    slab.free(slot);
}
