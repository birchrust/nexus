//! Integration tests for RcSlot and WeakSlot.

use std::cell::Cell;

// =============================================================================
// Test allocator setup
// =============================================================================

#[derive(Debug, PartialEq, Clone)]
pub struct Order {
    pub id: u64,
    pub price: f64,
}

mod bounded_rc {
    nexus_slab::bounded_rc_allocator!(super::Order);
}

mod unbounded_rc {
    nexus_slab::unbounded_rc_allocator!(super::Order);
}

// Drop tracking
thread_local! {
    static DROP_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug, Clone)]
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

mod drop_rc {
    nexus_slab::bounded_rc_allocator!(super::DropTracker);
}

/// Idempotent init — ignores AlreadyInitialized.
fn init_bounded_rc() {
    let _ = bounded_rc::Allocator::builder().capacity(64).build();
}

fn init_drop_rc() {
    let _ = drop_rc::Allocator::builder().capacity(64).build();
}

fn init_unbounded_rc() {
    let _ = unbounded_rc::Allocator::builder().chunk_size(8).build();
}

// =============================================================================
// Basic alloc / deref / drop
// =============================================================================

#[test]
fn basic_alloc_deref_drop() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 1, price: 100.0 });
    assert_eq!(rc.id, 1);
    assert_eq!(rc.price, 100.0);
    assert_eq!(rc.strong_count(), 1);
    assert_eq!(rc.weak_count(), 0);
    drop(rc);
}

// =============================================================================
// Clone bumps strong, drop decrements
// =============================================================================

#[test]
fn clone_bumps_strong() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 2, price: 50.0 });
    assert_eq!(rc.strong_count(), 1);

    let rc2 = rc.clone();
    assert_eq!(rc.strong_count(), 2);
    assert_eq!(rc2.strong_count(), 2);

    drop(rc2);
    assert_eq!(rc.strong_count(), 1);

    drop(rc);
}

// =============================================================================
// Last strong drop with outstanding weak: value dropped, slot NOT freed
// =============================================================================

#[test]
fn last_strong_with_weak_value_dropped_slot_held() {
    init_drop_rc();
    reset_drop_count();

    let rc = drop_rc::RcSlot::new(DropTracker(1));
    let weak = rc.downgrade();

    assert_eq!(rc.strong_count(), 1);
    assert_eq!(rc.weak_count(), 1);
    assert_eq!(get_drop_count(), 0);

    drop(rc);
    assert_eq!(get_drop_count(), 1); // Value dropped
    assert!(weak.upgrade().is_none()); // Can't upgrade
    assert_eq!(weak.strong_count(), 0);

    drop(weak);
}

// =============================================================================
// Weak upgrade succeeds while strong > 0
// =============================================================================

#[test]
fn weak_upgrade_succeeds() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 5, price: 75.0 });
    let weak = rc.downgrade();

    let upgraded = weak.upgrade().expect("should succeed");
    assert_eq!(upgraded.id, 5);
    assert_eq!(upgraded.strong_count(), 2);

    drop(upgraded);
    drop(rc);
    drop(weak);
}

// =============================================================================
// Weak upgrade returns None after last strong dropped
// =============================================================================

#[test]
fn weak_upgrade_returns_none() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 6, price: 30.0 });
    let weak = rc.downgrade();

    drop(rc);
    assert!(weak.upgrade().is_none());
    drop(weak);
}

// =============================================================================
// downgrade / upgrade roundtrip
// =============================================================================

#[test]
fn downgrade_upgrade_roundtrip() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 9, price: 99.0 });
    let weak = rc.downgrade();
    let rc2 = weak.upgrade().unwrap();

    assert_eq!(rc2.id, 9);
    assert_eq!(rc.strong_count(), 2);
    assert_eq!(rc.weak_count(), 1);

    drop(rc2);
    drop(weak);
    drop(rc);
}

// =============================================================================
// get_mut returns Some when unique
// =============================================================================

#[test]
fn get_mut_when_unique() {
    init_bounded_rc();

    let mut rc = bounded_rc::RcSlot::new(Order { id: 11, price: 1.0 });
    assert_eq!(rc.strong_count(), 1);
    assert_eq!(rc.weak_count(), 0);

    // Should succeed — we're the only reference
    let val = rc.get_mut().expect("should be unique");
    val.price = 999.0;

    assert_eq!(rc.price, 999.0);
    drop(rc);
}

// =============================================================================
// get_mut returns None when cloned
// =============================================================================

#[test]
fn get_mut_returns_none_when_cloned() {
    init_bounded_rc();

    let mut rc = bounded_rc::RcSlot::new(Order { id: 12, price: 1.0 });
    let _rc2 = rc.clone();

    assert_eq!(rc.strong_count(), 2);
    assert!(rc.get_mut().is_none());
}

// =============================================================================
// get_mut returns None when weak exists
// =============================================================================

#[test]
fn get_mut_returns_none_when_weak_exists() {
    init_bounded_rc();

    let mut rc = bounded_rc::RcSlot::new(Order { id: 13, price: 1.0 });
    let _weak = rc.downgrade();

    assert_eq!(rc.strong_count(), 1);
    assert_eq!(rc.weak_count(), 1);
    assert!(rc.get_mut().is_none());
}

// =============================================================================
// make_mut when unique (no clone)
// =============================================================================

#[test]
fn make_mut_when_unique() {
    init_bounded_rc();

    let mut rc = bounded_rc::RcSlot::new(Order { id: 14, price: 1.0 });

    {
        let val = rc.make_mut();
        val.price = 500.0;
    }

    assert_eq!(rc.price, 500.0);
    assert_eq!(rc.strong_count(), 1);
}

// =============================================================================
// make_mut clones when shared
// =============================================================================

#[test]
fn make_mut_clones_when_shared() {
    init_bounded_rc();

    let mut rc = bounded_rc::RcSlot::new(Order { id: 15, price: 1.0 });
    let rc2 = rc.clone();

    assert_eq!(rc.strong_count(), 2);

    // make_mut should clone into new slot
    {
        let val = rc.make_mut();
        val.price = 777.0;
    }

    // rc now points to a different slot with the modified value
    assert_eq!(rc.price, 777.0);
    assert_eq!(rc.strong_count(), 1);

    // rc2 still has the original value
    assert_eq!(rc2.price, 1.0);
    assert_eq!(rc2.strong_count(), 1);
}

// =============================================================================
// make_mut clones when weak exists
// =============================================================================

#[test]
fn make_mut_clones_when_weak_exists() {
    init_bounded_rc();

    let mut rc = bounded_rc::RcSlot::new(Order { id: 16, price: 1.0 });
    let weak = rc.downgrade();

    assert_eq!(rc.strong_count(), 1);
    assert_eq!(rc.weak_count(), 1);

    // make_mut should clone because weak exists
    {
        let val = rc.make_mut();
        val.price = 888.0;
    }

    // rc now unique in new slot
    assert_eq!(rc.price, 888.0);
    assert_eq!(rc.strong_count(), 1);
    assert_eq!(rc.weak_count(), 0);

    // weak still points to old slot (now zombie)
    assert!(weak.upgrade().is_none());
}

// =============================================================================
// get_mut_unchecked mutation
// =============================================================================

#[test]
fn get_mut_unchecked_mutation() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 17, price: 1.0 });
    assert_eq!(rc.strong_count(), 1);

    // SAFETY: strong_count == 1, weak_count == 0
    unsafe {
        let val = rc.get_mut_unchecked();
        val.price = 999.0;
    }

    assert_eq!(rc.price, 999.0);
    drop(rc);
}

// =============================================================================
// try_new returns Full when bounded at capacity
// =============================================================================

#[test]
fn try_new_returns_full() {
    mod small_rc {
        nexus_slab::bounded_rc_allocator!(u64);
    }

    let _ = small_rc::Allocator::builder().capacity(2).build();

    let rc1 = small_rc::RcSlot::new(1u64);
    let rc2 = small_rc::RcSlot::new(2u64);

    let result = small_rc::RcSlot::try_new(3u64);
    assert!(result.is_err());

    // Recover the value
    if let Err(full) = result {
        assert_eq!(full.into_inner(), 3u64);
    }

    drop(rc1);
    drop(rc2);

    // Now should succeed
    let rc3 = small_rc::RcSlot::try_new(4u64);
    assert!(rc3.is_ok());
}

// =============================================================================
// Drop order independence
// =============================================================================

#[test]
fn drop_order_independence() {
    init_drop_rc();
    reset_drop_count();

    let rc1 = drop_rc::RcSlot::new(DropTracker(100));
    let rc2 = rc1.clone();
    let rc3 = rc1.clone();
    let weak = rc1.downgrade();

    // Drop in various orders
    drop(rc2);
    assert_eq!(get_drop_count(), 0);

    drop(rc1);
    assert_eq!(get_drop_count(), 0);

    drop(rc3); // Last strong — value dropped
    assert_eq!(get_drop_count(), 1);

    assert!(weak.upgrade().is_none());
    drop(weak); // Last weak — slot freed
}

// =============================================================================
// Nested clones
// =============================================================================

#[test]
fn nested_clones() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 18, price: 1.0 });
    let rc2 = rc.clone();
    let rc3 = rc2.clone();
    let rc4 = rc3.clone();

    assert_eq!(rc.strong_count(), 4);

    drop(rc3);
    assert_eq!(rc.strong_count(), 3);

    drop(rc);
    assert_eq!(rc2.strong_count(), 2);

    drop(rc4);
    assert_eq!(rc2.strong_count(), 1);

    drop(rc2);
}

// =============================================================================
// Unbounded allocator
// =============================================================================

#[test]
fn unbounded_basic() {
    init_unbounded_rc();

    let rc = unbounded_rc::RcSlot::new(Order { id: 20, price: 200.0 });
    assert_eq!(rc.id, 20);

    let weak = rc.downgrade();
    let rc2 = weak.upgrade().unwrap();
    assert_eq!(rc2.id, 20);
    assert_eq!(rc.strong_count(), 2);

    drop(rc2);
    drop(rc);
    assert!(weak.upgrade().is_none());
    drop(weak);
}

// =============================================================================
// Multiple weaks
// =============================================================================

#[test]
fn multiple_weaks() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 19, price: 5.0 });
    let w1 = rc.downgrade();
    let w2 = rc.downgrade();
    let w3 = w1.clone();

    assert_eq!(rc.weak_count(), 3);

    drop(w1);
    assert_eq!(rc.weak_count(), 2);

    drop(rc);
    assert!(w2.upgrade().is_none());
    assert!(w3.upgrade().is_none());

    drop(w2);
    drop(w3); // Last weak — slot freed
}

// =============================================================================
// AsRef
// =============================================================================

#[test]
fn as_ref_works() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 21, price: 7.0 });
    let r: &Order = rc.as_ref();
    assert_eq!(r.id, 21);
    drop(rc);
}

// =============================================================================
// Debug
// =============================================================================

#[test]
fn debug_format() {
    init_bounded_rc();

    let rc = bounded_rc::RcSlot::new(Order { id: 22, price: 8.0 });
    let debug = format!("{:?}", rc);
    assert!(debug.contains("RcSlot"));
    assert!(debug.contains("strong"));

    let weak = rc.downgrade();
    let debug_w = format!("{:?}", weak);
    assert!(debug_w.contains("WeakSlot"));

    drop(weak);
    drop(rc);
}

// =============================================================================
// LocalStatic Debug (via BoxSlot, not RcSlot)
// =============================================================================

mod boxslot_for_local_static {
    nexus_slab::bounded_allocator!(super::Order);
}

fn init_boxslot() {
    let _ = boxslot_for_local_static::Allocator::builder()
        .capacity(16)
        .build();
}

#[test]
fn local_static_debug() {
    init_boxslot();

    let slot = boxslot_for_local_static::BoxSlot::new(Order { id: 23, price: 9.0 });
    let leaked = slot.leak();

    let debug = format!("{:?}", leaked);
    assert!(debug.contains("LocalStatic"));
    assert!(debug.contains("23")); // id should appear
}
