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
    let slab = BoundedSlab::<u64>::with_capacity(8);

    let slot = slab.alloc(42);
    assert_eq!(*slot, 42);
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

#[test]
fn miri_unbounded_basic() {
    let slab = UnboundedSlab::<u64>::with_chunk_capacity(4);

    let slot = slab.alloc(42);
    assert_eq!(*slot, 42);
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

#[test]
fn miri_multiple_inserts() {
    let slab = BoundedSlab::<u64>::with_capacity(8);

    let s1 = slab.alloc(1);
    let s2 = slab.alloc(2);
    let s3 = slab.alloc(3);

    assert_eq!(*s1, 1);
    assert_eq!(*s2, 2);
    assert_eq!(*s3, 3);

    // SAFETY: slots were allocated from this slab
    unsafe {
        slab.free(s1);
        slab.free(s2);
        slab.free(s3);
    }
}

#[test]
fn miri_slot_deref_mut() {
    let slab = BoundedSlab::<u64>::with_capacity(4);

    let mut slot = slab.alloc(42);
    *slot = 100;
    assert_eq!(*slot, 100);

    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

#[test]
fn miri_slot_replace() {
    let slab = BoundedSlab::<u64>::with_capacity(4);

    let mut slot = slab.alloc(1);
    let old = std::mem::replace(&mut *slot, 2);
    assert_eq!(old, 1);
    assert_eq!(*slot, 2);

    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

#[test]
fn miri_slot_into_inner() {
    let slab = BoundedSlab::<u64>::with_capacity(4);

    let slot = slab.alloc(42);
    // SAFETY: slot was allocated from this slab
    let value = unsafe { slab.take(slot) };
    assert_eq!(value, 42);
}

// =============================================================================
// Drop Safety
// =============================================================================

#[test]
fn miri_drop_on_slot_drop() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::with_capacity(4);

    {
        let slot = slab.alloc(DropTracker(1));
        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };
    }

    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_into_inner() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::with_capacity(4);

    let slot = slab.alloc(DropTracker(1));
    // SAFETY: slot was allocated from this slab
    let value = unsafe { slab.take(slot) };
    assert_eq!(get_drop_count(), 0);

    drop(value);
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_replace() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::with_capacity(4);

    let mut slot = slab.alloc(DropTracker(1));
    let old = std::mem::replace(&mut *slot, DropTracker(2));
    drop(old);
    assert_eq!(get_drop_count(), 1);

    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
    assert_eq!(get_drop_count(), 2);
}

#[test]
fn miri_no_drop_after_leak() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::with_capacity(4);

    let _slot = slab.alloc(DropTracker(1));
    // Intentionally leak - don't dealloc the slot

    assert_eq!(get_drop_count(), 0);
}

// =============================================================================
// Heap-Allocated Types
// =============================================================================

#[test]
fn miri_string_insert_drop() {
    let slab = BoundedSlab::<String>::with_capacity(4);

    let slot = slab.alloc("hello world".to_string());
    assert_eq!(*slot, "hello world");
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

#[test]
fn miri_vec_insert_drop() {
    // SAFETY: slab outlives all slots
    let slab = BoundedSlab::<Vec<u64>>::with_capacity(4);

    let slot = slab.alloc(vec![1, 2, 3, 4, 5]);
    assert_eq!(slot.len(), 5);
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

#[test]
fn miri_box_insert_drop() {
    // SAFETY: slab outlives all slots
    let slab = BoundedSlab::<Box<[u8; 1024]>>::with_capacity(4);

    let slot = slab.alloc(Box::new([0u8; 1024]));
    assert_eq!(slot.len(), 1024);
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

#[test]
fn miri_string_into_inner() {
    let slab = BoundedSlab::<String>::with_capacity(4);

    let slot = slab.alloc("hello".to_string());
    // SAFETY: slot was allocated from this slab
    let value = unsafe { slab.take(slot) };
    assert_eq!(value, "hello");
}

#[test]
fn miri_vec_replace() {
    // SAFETY: slab outlives all slots
    let slab = BoundedSlab::<Vec<u64>>::with_capacity(4);

    let mut slot = slab.alloc(vec![1, 2, 3]);
    let old = std::mem::replace(&mut *slot, vec![4, 5, 6, 7]);

    assert_eq!(old, vec![1, 2, 3]);
    assert_eq!(*slot, vec![4, 5, 6, 7]);

    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

// =============================================================================
// Slot Reuse
// =============================================================================

#[test]
fn miri_slot_reuse_bounded() {
    let slab = BoundedSlab::<String>::with_capacity(2);

    // Fill
    let s1 = slab.alloc("one".to_string());
    let s2 = slab.alloc("two".to_string());

    let p1 = s1.as_ptr();
    let p2 = s2.as_ptr();

    // Dealloc one
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(s1) };

    // Reuse
    let s3 = slab.alloc("three".to_string());
    assert_eq!(*s3, "three");
    assert_eq!(s3.as_ptr(), p1); // Reused slot 1

    // Dealloc other
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(s2) };

    // Reuse again
    let s4 = slab.alloc("four".to_string());
    assert_eq!(*s4, "four");
    assert_eq!(s4.as_ptr(), p2); // Reused slot 2

    // Clean up
    // SAFETY: slots were allocated from this slab
    unsafe {
        slab.free(s3);
        slab.free(s4);
    }
}

#[test]
fn miri_slot_reuse_single() {
    let slab = BoundedSlab::<String>::with_capacity(1);

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
        unsafe { slab.free(slot) };
    }
}

// =============================================================================
// Unbounded Growth
// =============================================================================

#[test]
fn miri_unbounded_growth() {
    let slab = UnboundedSlab::<u64>::with_chunk_capacity(4);

    // Fill multiple chunks
    let slots: Vec<_> = (0..12).map(|i| slab.alloc(i)).collect();

    assert!(slab.capacity() >= 12);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, i as u64);
    }

    // Clean up
    for slot in slots {
        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };
    }
}

#[test]
fn miri_unbounded_string_growth() {
    let slab = UnboundedSlab::<String>::with_chunk_capacity(4);

    let slots: Vec<_> = (0..12)
        .map(|i| slab.alloc(format!("string_{}", i)))
        .collect();

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, format!("string_{}", i));
    }

    // Clean up
    for slot in slots {
        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };
    }
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn miri_capacity_one() {
    let slab = BoundedSlab::<u64>::with_capacity(1);

    let slot = slab.alloc(42);
    assert!(slab.try_alloc(100).is_err());
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };

    let slot2 = slab.alloc(100);
    assert_eq!(*slot2, 100);
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot2) };
}

#[test]
fn miri_zst() {
    let slab = BoundedSlab::<ZeroSized>::with_capacity(10);

    let slot = slab.alloc(ZeroSized);
    assert_eq!(*slot, ZeroSized);
    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

#[test]
fn miri_large_struct() {
    let slab = BoundedSlab::<Large>::with_capacity(4);

    let mut data = [0u64; 128];
    for (i, d) in data.iter_mut().enumerate() {
        *d = i as u64;
    }

    let slot = slab.alloc(Large { data });
    assert_eq!(slot.data[0], 0);
    assert_eq!(slot.data[127], 127);

    // SAFETY: slot was allocated from this slab
    unsafe { slab.free(slot) };
}

// =============================================================================
// RC + Weak Miri Tests
// =============================================================================

mod miri_rc_alloc {
    nexus_slab::bounded_rc_allocator!(super::DropTracker);
}

mod miri_rc_u64 {
    nexus_slab::bounded_rc_allocator!(u64);
}

mod miri_rc_string {
    nexus_slab::bounded_rc_allocator!(String);
}

fn init_miri_rc_u64() {
    let _ = miri_rc_u64::Allocator::builder().capacity(16).build();
}

fn init_miri_rc_alloc() {
    let _ = miri_rc_alloc::Allocator::builder().capacity(16).build();
}

fn init_miri_rc_string() {
    let _ = miri_rc_string::Allocator::builder().capacity(16).build();
}

#[test]
fn miri_rc_basic_cycle() {
    init_miri_rc_u64();

    let rc = miri_rc_u64::RcSlot::try_new(42).unwrap();
    assert_eq!(*rc, 42);
    let rc2 = rc.clone();
    assert_eq!(*rc2, 42);
    drop(rc);
    drop(rc2);
}

#[test]
fn miri_rc_drop_tracker() {
    init_miri_rc_alloc();
    reset_drop_count();

    let rc = miri_rc_alloc::RcSlot::try_new(DropTracker(1)).unwrap();
    let rc2 = rc.clone();
    drop(rc);
    assert_eq!(get_drop_count(), 0);
    drop(rc2);
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_rc_weak_upgrade_downgrade() {
    init_miri_rc_u64();

    let rc = miri_rc_u64::RcSlot::try_new(99).unwrap();
    let weak = rc.downgrade();
    let upgraded = weak.upgrade().unwrap();
    assert_eq!(*upgraded, 99);
    drop(upgraded);
    drop(rc);
    assert!(weak.upgrade().is_none());
    drop(weak);
}

#[test]
fn miri_rc_zombie_slot() {
    init_miri_rc_alloc();
    reset_drop_count();

    let rc = miri_rc_alloc::RcSlot::try_new(DropTracker(2)).unwrap();
    let weak = rc.downgrade();
    drop(rc);
    assert_eq!(get_drop_count(), 1);

    // Zombie: value dropped, slot alive via weak
    assert!(weak.upgrade().is_none());
    drop(weak); // Slot freed
}

#[test]
fn miri_rc_string_type() {
    init_miri_rc_string();

    let rc = miri_rc_string::RcSlot::try_new("hello world".to_string()).unwrap();
    assert_eq!(&**rc, "hello world");
    let rc2 = rc.clone();
    drop(rc);
    assert_eq!(&**rc2, "hello world");
    drop(rc2);
}

#[test]
fn miri_rc_multiple_weaks_dealloc() {
    init_miri_rc_u64();

    let rc = miri_rc_u64::RcSlot::try_new(7).unwrap();
    let w1 = rc.downgrade();
    let w2 = rc.downgrade();
    let w3 = w1.clone();

    drop(rc);
    drop(w1);
    drop(w3);
    drop(w2); // Last weak — slot freed
}

#[test]
fn miri_rc_slot_reuse_after_weak() {
    init_miri_rc_u64();

    let rc = miri_rc_u64::RcSlot::try_new(10).unwrap();
    let weak = rc.downgrade();
    drop(rc);
    drop(weak);

    // Slot should be reusable — if not, this would fail on bounded allocator
    let rc2 = miri_rc_u64::RcSlot::try_new(20).unwrap();
    assert_eq!(*rc2, 20);
    drop(rc2);
}

#[test]
fn miri_rc_get_mut() {
    init_miri_rc_u64();

    let mut rc = miri_rc_u64::RcSlot::try_new(100).unwrap();

    // get_mut succeeds when unique
    if let Some(val) = rc.get_mut() {
        *val = 200;
    }
    assert_eq!(*rc, 200);

    // get_mut fails when cloned
    let _rc2 = rc.clone();
    assert!(rc.get_mut().is_none());
}
