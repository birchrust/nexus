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
    let slab = BoundedSlab::<u64>::new(8);

    let slot = slab.new_slot(42);
    assert_eq!(*slot, 42);
    drop(slot);
}

#[test]
fn miri_unbounded_basic() {
    let slab = UnboundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    assert_eq!(*slot, 42);
    drop(slot);
}

#[test]
fn miri_multiple_inserts() {
    let slab = BoundedSlab::<u64>::new(8);

    let s1 = slab.new_slot(1);
    let s2 = slab.new_slot(2);
    let s3 = slab.new_slot(3);

    assert_eq!(*s1, 1);
    assert_eq!(*s2, 2);
    assert_eq!(*s3, 3);
}

#[test]
fn miri_slot_deref_mut() {
    let slab = BoundedSlab::<u64>::new(4);

    let mut slot = slab.new_slot(42);
    *slot = 100;
    assert_eq!(*slot, 100);
}

#[test]
fn miri_slot_replace() {
    let slab = BoundedSlab::<u64>::new(4);

    let mut slot = slab.new_slot(1);
    let old = slot.replace(2);
    assert_eq!(old, 1);
    assert_eq!(*slot, 2);
}

#[test]
fn miri_slot_into_inner() {
    let slab = BoundedSlab::<u64>::new(4);

    let slot = slab.new_slot(42);
    let value = slot.into_inner();
    assert_eq!(value, 42);
}

// =============================================================================
// Drop Safety
// =============================================================================

#[test]
fn miri_drop_on_slot_drop() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    {
        let _slot = slab.new_slot(DropTracker(1));
    }

    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_into_inner() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    let slot = slab.new_slot(DropTracker(1));
    let value = slot.into_inner();
    assert_eq!(get_drop_count(), 0);

    drop(value);
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_drop_on_replace() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    let mut slot = slab.new_slot(DropTracker(1));
    let old = slot.replace(DropTracker(2));
    drop(old);
    assert_eq!(get_drop_count(), 1);

    drop(slot);
    assert_eq!(get_drop_count(), 2);
}

#[test]
fn miri_no_drop_after_leak() {
    reset_drop_count();

    let slab = BoundedSlab::<DropTracker>::new(4);

    let slot = slab.new_slot(DropTracker(1));
    let _key = slot.leak();

    assert_eq!(get_drop_count(), 0);
}

// =============================================================================
// Heap-Allocated Types
// =============================================================================

#[test]
fn miri_string_insert_drop() {
    let slab = BoundedSlab::<String>::new(4);

    let slot = slab.new_slot("hello world".to_string());
    assert_eq!(*slot, "hello world");
    drop(slot);
}

#[test]
fn miri_vec_insert_drop() {
    let slab = BoundedSlab::<Vec<u64>>::new(4);

    let slot = slab.new_slot(vec![1, 2, 3, 4, 5]);
    assert_eq!(slot.len(), 5);
    drop(slot);
}

#[test]
fn miri_box_insert_drop() {
    let slab = BoundedSlab::<Box<[u8; 1024]>>::new(4);

    let slot = slab.new_slot(Box::new([0u8; 1024]));
    assert_eq!(slot.len(), 1024);
    drop(slot);
}

#[test]
fn miri_string_into_inner() {
    let slab = BoundedSlab::<String>::new(4);

    let slot = slab.new_slot("hello".to_string());
    let value = slot.into_inner();
    assert_eq!(value, "hello");
}

#[test]
fn miri_vec_replace() {
    let slab = BoundedSlab::<Vec<u64>>::new(4);

    let mut slot = slab.new_slot(vec![1, 2, 3]);
    let old = slot.replace(vec![4, 5, 6, 7]);

    assert_eq!(old, vec![1, 2, 3]);
    assert_eq!(*slot, vec![4, 5, 6, 7]);
}

// =============================================================================
// Slot Reuse
// =============================================================================

#[test]
fn miri_slot_reuse_bounded() {
    let slab = BoundedSlab::<String>::new(2);

    // Fill
    let s1 = slab.new_slot("one".to_string());
    let s2 = slab.new_slot("two".to_string());

    let k1 = s1.key();
    let k2 = s2.key();

    // Free one
    drop(s1);

    // Reuse
    let s3 = slab.new_slot("three".to_string());
    assert_eq!(*s3, "three");
    assert_eq!(s3.key(), k1); // Reused slot 1

    // Free other
    drop(s2);

    // Reuse again
    let s4 = slab.new_slot("four".to_string());
    assert_eq!(*s4, "four");
    assert_eq!(s4.key(), k2); // Reused slot 2
}

#[test]
fn miri_slot_reuse_single() {
    let slab = BoundedSlab::<String>::new(1);

    for i in 0..10 {
        let slot = slab.new_slot(format!("value_{}", i));
        assert_eq!(*slot, format!("value_{}", i));
        assert_eq!(slot.key().index(), 0);
    }
}

// =============================================================================
// Unbounded Growth
// =============================================================================

#[test]
fn miri_unbounded_growth() {
    let slab = UnboundedSlab::<u64>::new(4);

    // Fill multiple chunks
    let slots: Vec<_> = (0..12).map(|i| slab.new_slot(i)).collect();

    assert!(slab.capacity() >= 12);

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, i as u64);
    }
}

#[test]
fn miri_unbounded_string_growth() {
    let slab = UnboundedSlab::<String>::new(4);

    let slots: Vec<_> = (0..12)
        .map(|i| slab.new_slot(format!("string_{}", i)))
        .collect();

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(**slot, format!("string_{}", i));
    }
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn miri_capacity_one() {
    let slab = BoundedSlab::<u64>::new(1);

    let slot = slab.new_slot(42);
    assert!(slab.try_new_slot(100).is_err());
    drop(slot);

    let slot2 = slab.new_slot(100);
    assert_eq!(*slot2, 100);
}

#[test]
fn miri_zst() {
    let slab = BoundedSlab::<ZeroSized>::new(10);

    let slot = slab.new_slot(ZeroSized);
    assert_eq!(*slot, ZeroSized);
    drop(slot);
}

#[test]
fn miri_large_struct() {
    let slab = BoundedSlab::<Large>::new(4);

    let mut data = [0u64; 128];
    for (i, d) in data.iter_mut().enumerate() {
        *d = i as u64;
    }

    let slot = slab.new_slot(Large { data });
    assert_eq!(slot.data[0], 0);
    assert_eq!(slot.data[127], 127);
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

    let rc = miri_rc_u64::RcSlot::new(42);
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

    let rc = miri_rc_alloc::RcSlot::new(DropTracker(1));
    let rc2 = rc.clone();
    drop(rc);
    assert_eq!(get_drop_count(), 0);
    drop(rc2);
    assert_eq!(get_drop_count(), 1);
}

#[test]
fn miri_rc_weak_upgrade_downgrade() {
    init_miri_rc_u64();

    let rc = miri_rc_u64::RcSlot::new(99);
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

    let rc = miri_rc_alloc::RcSlot::new(DropTracker(2));
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

    let rc = miri_rc_string::RcSlot::new("hello world".to_string());
    assert_eq!(&**rc, "hello world");
    let rc2 = rc.clone();
    drop(rc);
    assert_eq!(&**rc2, "hello world");
    drop(rc2);
}

#[test]
fn miri_rc_multiple_weaks_dealloc() {
    init_miri_rc_u64();

    let rc = miri_rc_u64::RcSlot::new(7);
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

    let rc = miri_rc_u64::RcSlot::new(10);
    let weak = rc.downgrade();
    drop(rc);
    drop(weak);

    // Slot should be reusable — if not, this would fail on bounded allocator
    let rc2 = miri_rc_u64::RcSlot::new(20);
    assert_eq!(*rc2, 20);
    drop(rc2);
}

#[test]
fn miri_rc_leak_local_static() {
    init_miri_rc_u64();

    let rc = miri_rc_u64::RcSlot::new(42);
    let leaked = rc.leak();

    // LocalStatic dereferences correctly
    assert_eq!(*leaked, 42);

    // LocalStatic is Copy
    let leaked2 = leaked;
    assert_eq!(*leaked, *leaked2);
}

#[test]
fn miri_rc_get_mut() {
    init_miri_rc_u64();

    let mut rc = miri_rc_u64::RcSlot::new(100);

    // get_mut succeeds when unique
    if let Some(val) = rc.get_mut() {
        *val = 200;
    }
    assert_eq!(*rc, 200);

    // get_mut fails when cloned
    let _rc2 = rc.clone();
    assert!(rc.get_mut().is_none());
}
