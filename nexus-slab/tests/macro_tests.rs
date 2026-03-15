//! Tests for the bounded_allocator! and unbounded_allocator! macros.

#![allow(clippy::float_cmp)]

use nexus_slab::Alloc;

// =============================================================================
// Test type
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub id: u64,
    pub price: f64,
}

impl Order {
    fn new(id: u64, price: f64) -> Self {
        Self { id, price }
    }
}

// =============================================================================
// Create test allocator modules
// =============================================================================

mod order_alloc {
    nexus_slab::bounded_allocator!(super::Order);
}

// =============================================================================
// Tests
// =============================================================================

#[test]
fn test_basic_alloc_dealloc() {
    order_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    assert!(order_alloc::Allocator::is_initialized());
    assert_eq!(order_alloc::Allocator::capacity(), 10);

    let slot = order_alloc::BoxSlot::try_new(Order::new(1, 100.0)).unwrap();

    // Deref works
    assert_eq!(slot.id, 1);
    assert_eq!(slot.price, 100.0);

    drop(slot);
}

#[test]
fn test_deref_mut() {
    order_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let mut slot = order_alloc::BoxSlot::try_new(Order::new(1, 100.0)).unwrap();

    // Mutate through DerefMut
    slot.price = 200.0;
    assert_eq!(slot.price, 200.0);

    drop(slot);
}

#[test]
fn test_into_inner() {
    order_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = order_alloc::BoxSlot::try_new(Order::new(42, 99.99)).unwrap();

    let order = slot.into_inner();
    assert_eq!(order.id, 42);
    assert_eq!(order.price, 99.99);
}

#[test]
fn test_into_slot_and_from_slot() {
    order_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = order_alloc::BoxSlot::try_new(Order::new(123, 456.78)).unwrap();

    // into_slot() returns the raw RawSlot<T> for manual management
    let raw_slot = slot.into_slot();
    let ptr = raw_slot.as_ptr();

    // Access via from_slot (reconstructs RAII handle from raw slot)
    let order = unsafe { order_alloc::BoxSlot::from_slot(raw_slot) };
    assert_eq!(order.id, 123);

    // Verify it's the same slot by going through into_slot again
    let raw_slot2 = order.into_slot();
    assert_eq!(raw_slot2.as_ptr(), ptr);

    // Restore and drop
    let restored = unsafe { order_alloc::BoxSlot::from_slot(raw_slot2) };
    drop(restored);
}

#[test]
fn test_leak_returns_local_static() {
    order_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = order_alloc::BoxSlot::try_new(Order::new(42, 99.99)).unwrap();

    // leak() returns LocalStatic<T> - immutable, permanent reference
    let leaked: nexus_slab::LocalStatic<Order> = slot.leak();

    // Can read via Deref
    assert_eq!(leaked.id, 42);
    assert_eq!(leaked.price, 99.99);

    // LocalStatic is Copy
    let leaked2 = leaked;
    assert_eq!(leaked2.id, 42);
}

// Each drop-tracking test gets its own module to avoid global counter races
mod drop_called_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    pub struct Tracker(#[allow(dead_code)] pub u64);
    impl Drop for Tracker {
        fn drop(&mut self) {
            COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn reset() {
        COUNT.store(0, Ordering::SeqCst);
    }
    pub fn count() -> usize {
        COUNT.load(Ordering::SeqCst)
    }

    pub mod alloc {
        nexus_slab::bounded_allocator!(super::Tracker);
    }
}

mod drop_leak_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    pub struct Tracker(#[allow(dead_code)] pub u64);
    impl Drop for Tracker {
        fn drop(&mut self) {
            COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn reset() {
        COUNT.store(0, Ordering::SeqCst);
    }
    pub fn count() -> usize {
        COUNT.load(Ordering::SeqCst)
    }

    pub mod alloc {
        nexus_slab::bounded_allocator!(super::Tracker);
    }
}

#[test]
fn test_drop_called() {
    drop_called_test::reset();

    drop_called_test::alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    {
        let _slot =
            drop_called_test::alloc::BoxSlot::try_new(drop_called_test::Tracker(1)).unwrap();
        assert_eq!(drop_called_test::count(), 0);
    }

    assert_eq!(drop_called_test::count(), 1);
}

#[test]
fn test_drop_not_called_after_into_slot() {
    drop_leak_test::reset();

    drop_leak_test::alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = drop_leak_test::alloc::BoxSlot::try_new(drop_leak_test::Tracker(1)).unwrap();
    let raw_slot = slot.into_slot(); // into_slot() returns raw RawSlot<T>, discarding RAII

    assert_eq!(drop_leak_test::count(), 0);

    // Manual cleanup using from_slot to reconstruct RAII handle
    let restored = unsafe { drop_leak_test::alloc::BoxSlot::from_slot(raw_slot) };
    drop(restored);
    assert_eq!(drop_leak_test::count(), 1); // Dropped when restored is dropped
}

#[test]
fn test_capacity_full() {
    order_alloc::Allocator::builder()
        .capacity(2)
        .build()
        .expect("init should succeed");

    let slot1 = order_alloc::BoxSlot::try_new(Order::new(1, 1.0)).unwrap();
    let slot2 = order_alloc::BoxSlot::try_new(Order::new(2, 2.0)).unwrap();

    // Should fail - use try_new to get Full(value) back
    let result = order_alloc::BoxSlot::try_new(Order::new(3, 3.0));
    assert!(result.is_err());
    let recovered = result.unwrap_err().into_inner();
    assert_eq!(recovered.id, 3);

    drop(slot1);

    // Now should succeed
    let slot3 = order_alloc::BoxSlot::try_new(Order::new(3, 3.0)).unwrap();
    assert_eq!(slot3.id, 3);

    drop(slot2);
    drop(slot3);
}

#[test]
fn test_already_initialized_error() {
    // Note: This test uses a separate module to avoid conflicts with other tests
    mod local_alloc {
        nexus_slab::bounded_allocator!(super::Order);
    }

    local_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("first init should succeed");

    let result = local_alloc::Allocator::builder().capacity(20).build();

    assert!(result.is_err());
}

#[test]
fn test_borrow_traits() {
    use std::borrow::{Borrow, BorrowMut};

    order_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let mut slot = order_alloc::BoxSlot::try_new(Order::new(1, 100.0)).unwrap();

    // Borrow
    let borrowed: &Order = slot.borrow();
    assert_eq!(borrowed.id, 1);

    // BorrowMut
    let borrowed_mut: &mut Order = slot.borrow_mut();
    borrowed_mut.price = 200.0;
    assert_eq!(slot.price, 200.0);

    // AsRef/AsMut
    let as_ref: &Order = slot.as_ref();
    assert_eq!(as_ref.id, 1);

    drop(slot);
}

// =============================================================================
// Unbounded allocator modules
// =============================================================================

mod unbounded_order_alloc {
    nexus_slab::unbounded_allocator!(super::Order);
}

// =============================================================================
// Unbounded allocator tests
// =============================================================================

#[test]
fn test_unbounded_basic_alloc_dealloc() {
    unbounded_order_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    assert!(unbounded_order_alloc::Allocator::is_initialized());

    // Unbounded BoxSlot::new always succeeds (no Option)
    let slot = unbounded_order_alloc::BoxSlot::new(Order::new(1, 100.0));

    // Deref works
    assert_eq!(slot.id, 1);
    assert_eq!(slot.price, 100.0);

    drop(slot);
}

#[test]
fn test_unbounded_deref_mut() {
    unbounded_order_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let mut slot = unbounded_order_alloc::BoxSlot::new(Order::new(1, 100.0));

    // Mutate through DerefMut
    slot.price = 200.0;
    assert_eq!(slot.price, 200.0);

    drop(slot);
}

#[test]
fn test_unbounded_into_inner() {
    unbounded_order_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let slot = unbounded_order_alloc::BoxSlot::new(Order::new(42, 99.99));

    let order = slot.into_inner();
    assert_eq!(order.id, 42);
    assert_eq!(order.price, 99.99);
}

#[test]
fn test_unbounded_into_slot_and_from_slot() {
    unbounded_order_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let slot = unbounded_order_alloc::BoxSlot::new(Order::new(123, 456.78));

    // into_slot() returns the raw RawSlot<T> for manual management
    let raw_slot = slot.into_slot();
    let ptr = raw_slot.as_ptr();

    // Access via from_slot (reconstructs RAII handle from raw slot)
    let order = unsafe { unbounded_order_alloc::BoxSlot::from_slot(raw_slot) };
    assert_eq!(order.id, 123);

    // Verify it's the same slot by going through into_slot again
    let raw_slot2 = order.into_slot();
    assert_eq!(raw_slot2.as_ptr(), ptr);

    // Restore and drop
    let restored = unsafe { unbounded_order_alloc::BoxSlot::from_slot(raw_slot2) };
    drop(restored);
}

#[test]
fn test_unbounded_leak_returns_local_static() {
    unbounded_order_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let slot = unbounded_order_alloc::BoxSlot::new(Order::new(42, 99.99));

    // leak() returns LocalStatic<T> - immutable, permanent reference
    let leaked: nexus_slab::LocalStatic<Order> = slot.leak();

    // Can read via Deref
    assert_eq!(leaked.id, 42);
    assert_eq!(leaked.price, 99.99);

    // LocalStatic is Copy
    let leaked2 = leaked;
    assert_eq!(leaked2.id, 42);
}

mod unbounded_drop_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    pub struct Tracker(#[allow(dead_code)] pub u64);
    impl Drop for Tracker {
        fn drop(&mut self) {
            COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn reset() {
        COUNT.store(0, Ordering::SeqCst);
    }
    pub fn count() -> usize {
        COUNT.load(Ordering::SeqCst)
    }

    pub mod alloc {
        nexus_slab::unbounded_allocator!(super::Tracker);
    }
}

#[test]
fn test_unbounded_drop_called() {
    unbounded_drop_test::reset();

    unbounded_drop_test::alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    {
        let _slot = unbounded_drop_test::alloc::BoxSlot::new(unbounded_drop_test::Tracker(1));
        assert_eq!(unbounded_drop_test::count(), 0);
    }

    assert_eq!(unbounded_drop_test::count(), 1);
}

#[test]
fn test_unbounded_grows_automatically() {
    mod local_alloc {
        nexus_slab::unbounded_allocator!(super::Order);
    }

    local_alloc::Allocator::builder()
        .chunk_size(4) // Small chunks to test growth
        .build()
        .expect("init should succeed");

    // Allocate more than chunk_size to force growth
    let mut slots = Vec::new();
    for i in 0..10 {
        let slot = local_alloc::BoxSlot::new(Order::new(i, i as f64));
        slots.push(slot);
    }

    // Capacity should have grown (at least 3 chunks of 4 = 12)
    assert!(local_alloc::Allocator::capacity() >= 10);
    assert_eq!(slots.len(), 10);

    // Drop all slots
    slots.clear();
}

#[test]
fn test_unbounded_chunk_freelist_maintenance() {
    mod local_alloc {
        nexus_slab::unbounded_allocator!(super::Order);
    }

    local_alloc::Allocator::builder()
        .chunk_size(2) // Very small chunks
        .build()
        .expect("init should succeed");

    // Fill first chunk
    let slot1 = local_alloc::BoxSlot::new(Order::new(1, 1.0));
    let slot2 = local_alloc::BoxSlot::new(Order::new(2, 2.0));
    // This should trigger growth to second chunk
    let slot3 = local_alloc::BoxSlot::new(Order::new(3, 3.0));

    // Free slot from first chunk - should add it back to available list
    drop(slot1);

    // Next allocation should reuse the freed slot in first chunk
    let slot4 = local_alloc::BoxSlot::new(Order::new(4, 4.0));

    drop(slot2);
    drop(slot3);
    drop(slot4);
}

#[test]
fn test_unbounded_already_initialized_error() {
    mod local_alloc {
        nexus_slab::unbounded_allocator!(super::Order);
    }

    local_alloc::Allocator::builder()
        .build()
        .expect("first init should succeed");

    let result = local_alloc::Allocator::builder().build();
    assert!(result.is_err());
}

#[test]
fn test_unbounded_borrow_traits() {
    use std::borrow::{Borrow, BorrowMut};

    unbounded_order_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let mut slot = unbounded_order_alloc::BoxSlot::new(Order::new(1, 100.0));

    // Borrow
    let borrowed: &Order = slot.borrow();
    assert_eq!(borrowed.id, 1);

    // BorrowMut
    let borrowed_mut: &mut Order = slot.borrow_mut();
    borrowed_mut.price = 200.0;
    assert_eq!(slot.price, 200.0);

    // AsRef/AsMut
    let as_ref: &Order = slot.as_ref();
    assert_eq!(as_ref.id, 1);

    drop(slot);
}

// =============================================================================
// Trait assertion tests
// =============================================================================

fn assert_bounded<A: nexus_slab::BoundedAlloc>() {}
fn assert_unbounded<A: nexus_slab::UnboundedAlloc>() {}
fn assert_slab_allocator<A: nexus_slab::Alloc>() {}

#[test]
fn test_bounded_trait_marker() {
    assert_bounded::<order_alloc::Allocator>();
    assert_slab_allocator::<order_alloc::Allocator>();
}

#[test]
fn test_unbounded_trait_marker() {
    assert_unbounded::<unbounded_order_alloc::Allocator>();
    assert_slab_allocator::<unbounded_order_alloc::Allocator>();
}

#[test]
fn test_slot_size_is_8_bytes() {
    assert_eq!(
        std::mem::size_of::<order_alloc::BoxSlot>(),
        8,
        "Slot<A> should be 8 bytes (one pointer)"
    );
    assert_eq!(
        std::mem::size_of::<unbounded_order_alloc::BoxSlot>(),
        8,
        "Slot<A> should be 8 bytes (one pointer)"
    );
}
