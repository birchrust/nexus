//! Tests for bounded_byte_allocator! and unbounded_byte_allocator! macros.

#![allow(clippy::float_cmp, clippy::derive_partial_eq_without_eq)]

use nexus_slab::Alloc;
use std::borrow::{Borrow, BorrowMut};

// =============================================================================
// Test types — multiple types stored in the SAME byte slab
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

#[derive(Debug, Clone, PartialEq)]
pub struct Cancel {
    pub id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZeroSized;

// =============================================================================
// Bounded byte allocator — 64-byte slots
// =============================================================================

mod bounded_alloc {
    nexus_slab::bounded_byte_allocator!(64);
}

// =============================================================================
// Bounded: Basic operations
// =============================================================================

#[test]
fn bounded_basic_alloc_dealloc() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    assert!(bounded_alloc::Allocator::is_initialized());
    assert_eq!(bounded_alloc::Allocator::capacity(), 10);

    let slot = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(1, 100.0)).unwrap();
    assert_eq!(slot.id, 1);
    assert_eq!(slot.price, 100.0);
    drop(slot);
}

#[test]
fn bounded_multiple_types_same_slab() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let order = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(1, 100.0)).unwrap();
    let cancel = bounded_alloc::BoxSlot::<Cancel>::try_new(Cancel { id: 2 }).unwrap();
    let num = bounded_alloc::BoxSlot::<u64>::try_new(42u64).unwrap();

    assert_eq!(order.id, 1);
    assert_eq!(cancel.id, 2);
    assert_eq!(*num, 42);

    drop(order);
    drop(cancel);
    drop(num);
}

#[test]
fn bounded_deref_mut() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let mut slot = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(1, 100.0)).unwrap();
    slot.price = 200.0;
    assert_eq!(slot.price, 200.0);
    drop(slot);
}

#[test]
fn bounded_into_inner() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(42, 99.99)).unwrap();
    let order = slot.into_inner();
    assert_eq!(order.id, 42);
    assert_eq!(order.price, 99.99);
}

#[test]
fn bounded_replace() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let mut slot = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(1, 100.0)).unwrap();
    let old = slot.replace(Order::new(2, 200.0));
    assert_eq!(old, Order::new(1, 100.0));
    assert_eq!(slot.id, 2);
    assert_eq!(slot.price, 200.0);
    drop(slot);
}

#[test]
fn bounded_leak_returns_local_static() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(42, 99.99)).unwrap();
    let leaked: nexus_slab::LocalStatic<Order> = slot.leak();

    assert_eq!(leaked.id, 42);
    assert_eq!(leaked.price, 99.99);

    // LocalStatic is Copy
    let leaked2 = leaked;
    assert_eq!(leaked2.id, 42);
}

#[test]
fn bounded_capacity_full() {
    bounded_alloc::Allocator::builder()
        .capacity(2)
        .build()
        .expect("init should succeed");

    let slot1 = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(1, 1.0)).unwrap();
    let slot2 = bounded_alloc::BoxSlot::<Cancel>::try_new(Cancel { id: 2 }).unwrap();

    // Full — returns value back
    let result = bounded_alloc::BoxSlot::<u64>::try_new(99u64);
    assert!(result.is_err());
    let recovered = result.unwrap_err().into_inner();
    assert_eq!(recovered, 99);

    drop(slot1);

    // Now succeeds
    let slot3 = bounded_alloc::BoxSlot::<u64>::try_new(3u64).unwrap();
    assert_eq!(*slot3, 3);

    drop(slot2);
    drop(slot3);
}

#[test]
fn bounded_already_initialized_error() {
    mod local_alloc {
        nexus_slab::bounded_byte_allocator!(64);
    }

    local_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("first init should succeed");

    let result = local_alloc::Allocator::builder().capacity(20).build();
    assert!(result.is_err());
}

#[test]
fn bounded_borrow_traits() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let mut slot = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(1, 100.0)).unwrap();

    let borrowed: &Order = slot.borrow();
    assert_eq!(borrowed.id, 1);

    let borrowed_mut: &mut Order = slot.borrow_mut();
    borrowed_mut.price = 200.0;
    assert_eq!(slot.price, 200.0);

    let as_ref: &Order = slot.as_ref();
    assert_eq!(as_ref.id, 1);

    drop(slot);
}

#[test]
fn bounded_pin_stable_address() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(1, 100.0)).unwrap();
    let addr = &raw const *slot;

    // pin() returns Pin<&T> at the same address
    let pinned = slot.pin();
    assert_eq!(&raw const *pinned, addr);

    drop(slot);
}

#[test]
fn bounded_pin_mut_stable_address() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let mut slot = bounded_alloc::BoxSlot::<Order>::try_new(Order::new(1, 100.0)).unwrap();
    let addr = &raw const *slot;

    // pin_mut() returns Pin<&mut T> at the same address
    let pinned = slot.pin_mut();
    assert_eq!(&raw const *pinned, addr);

    drop(slot);
}

#[test]
fn bounded_zst() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = bounded_alloc::BoxSlot::<ZeroSized>::try_new(ZeroSized).unwrap();
    assert_eq!(*slot, ZeroSized);
    drop(slot);
}

#[test]
fn bounded_string_type() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = bounded_alloc::BoxSlot::<String>::try_new("hello world".to_string()).unwrap();
    assert_eq!(&*slot, "hello world");
    drop(slot);
}

#[test]
fn bounded_debug_format() {
    bounded_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = bounded_alloc::BoxSlot::<u64>::try_new(42u64).unwrap();
    let debug = format!("{:?}", slot);
    assert!(debug.contains("BoxSlot"));
    assert!(debug.contains("42"));
    drop(slot);
}

// =============================================================================
// Bounded: Drop tracking
// =============================================================================

mod bounded_drop_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[allow(dead_code)]
    pub struct Tracker(pub u64);
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
        nexus_slab::bounded_byte_allocator!(64);
    }
}

#[test]
fn bounded_drop_called() {
    bounded_drop_test::reset();
    bounded_drop_test::alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    {
        let _slot = bounded_drop_test::alloc::BoxSlot::<bounded_drop_test::Tracker>::try_new(
            bounded_drop_test::Tracker(1),
        )
        .unwrap();
        assert_eq!(bounded_drop_test::count(), 0);
    }

    assert_eq!(bounded_drop_test::count(), 1);
}

mod bounded_drop_multi_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[allow(dead_code)]
    pub struct Tracker(pub u64);
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
        nexus_slab::bounded_byte_allocator!(64);
    }
}

#[test]
fn bounded_drop_multiple_types() {
    bounded_drop_multi_test::reset();
    bounded_drop_multi_test::alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    {
        let _t1 =
            bounded_drop_multi_test::alloc::BoxSlot::<bounded_drop_multi_test::Tracker>::try_new(
                bounded_drop_multi_test::Tracker(1),
            )
            .unwrap();
        // Also store a non-tracked type in the same slab
        let _num = bounded_drop_multi_test::alloc::BoxSlot::<u64>::try_new(42u64).unwrap();
        let _t2 =
            bounded_drop_multi_test::alloc::BoxSlot::<bounded_drop_multi_test::Tracker>::try_new(
                bounded_drop_multi_test::Tracker(2),
            )
            .unwrap();

        assert_eq!(bounded_drop_multi_test::count(), 0);
    }

    // Only the 2 Trackers should have been dropped (u64 has no Drop)
    assert_eq!(bounded_drop_multi_test::count(), 2);
}

mod bounded_drop_into_inner_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[allow(dead_code)]
    pub struct Tracker(pub u64);
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
        nexus_slab::bounded_byte_allocator!(64);
    }
}

#[test]
fn bounded_into_inner_does_not_double_drop() {
    bounded_drop_into_inner_test::reset();
    bounded_drop_into_inner_test::alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = bounded_drop_into_inner_test::alloc::BoxSlot::<
        bounded_drop_into_inner_test::Tracker,
    >::try_new(bounded_drop_into_inner_test::Tracker(1))
    .unwrap();

    assert_eq!(bounded_drop_into_inner_test::count(), 0);
    let value = slot.into_inner();
    // into_inner extracts the value — no drop yet
    assert_eq!(bounded_drop_into_inner_test::count(), 0);
    drop(value);
    // Now it drops
    assert_eq!(bounded_drop_into_inner_test::count(), 1);
}

mod bounded_drop_replace_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[allow(dead_code)]
    pub struct Tracker(pub u64);
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
        nexus_slab::bounded_byte_allocator!(64);
    }
}

#[test]
fn bounded_replace_drops_old_value() {
    bounded_drop_replace_test::reset();
    bounded_drop_replace_test::alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let mut slot =
        bounded_drop_replace_test::alloc::BoxSlot::<bounded_drop_replace_test::Tracker>::try_new(
            bounded_drop_replace_test::Tracker(1),
        )
        .unwrap();

    let old = slot.replace(bounded_drop_replace_test::Tracker(2));
    // Old value was moved out, not dropped yet
    assert_eq!(bounded_drop_replace_test::count(), 0);
    drop(old);
    // Old value dropped
    assert_eq!(bounded_drop_replace_test::count(), 1);
    drop(slot);
    // New value dropped
    assert_eq!(bounded_drop_replace_test::count(), 2);
}

mod bounded_drop_leak_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[allow(dead_code)]
    pub struct Tracker(pub u64);
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
        nexus_slab::bounded_byte_allocator!(64);
    }
}

#[test]
fn bounded_leak_does_not_drop() {
    bounded_drop_leak_test::reset();
    bounded_drop_leak_test::alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init should succeed");

    let slot = bounded_drop_leak_test::alloc::BoxSlot::<bounded_drop_leak_test::Tracker>::try_new(
        bounded_drop_leak_test::Tracker(1),
    )
    .unwrap();

    let _leaked = slot.leak();
    assert_eq!(bounded_drop_leak_test::count(), 0);
}

// =============================================================================
// Bounded: Stress
// =============================================================================

#[test]
fn bounded_stress_fill_drain_mixed_types() {
    mod local_alloc {
        nexus_slab::bounded_byte_allocator!(64);
    }

    local_alloc::Allocator::builder()
        .capacity(100)
        .build()
        .expect("init should succeed");

    for cycle in 0..10u64 {
        let mut order_slots = Vec::new();
        let mut cancel_slots = Vec::new();

        // Alternate types
        for i in 0..50u64 {
            order_slots.push(
                local_alloc::BoxSlot::<Order>::try_new(Order::new(cycle * 100 + i, i as f64))
                    .unwrap(),
            );
            cancel_slots.push(
                local_alloc::BoxSlot::<Cancel>::try_new(Cancel {
                    id: cycle * 100 + i + 50,
                })
                .unwrap(),
            );
        }

        // Verify values
        for (i, slot) in order_slots.iter().enumerate() {
            assert_eq!(slot.id, cycle * 100 + i as u64);
        }
        for (i, slot) in cancel_slots.iter().enumerate() {
            assert_eq!(slot.id, cycle * 100 + i as u64 + 50);
        }

        // Drain
        drop(order_slots);
        drop(cancel_slots);
    }
}

// =============================================================================
// Unbounded byte allocator — 64-byte slots
// =============================================================================

mod unbounded_alloc {
    nexus_slab::unbounded_byte_allocator!(64);
}

// =============================================================================
// Unbounded: Basic operations
// =============================================================================

#[test]
fn unbounded_basic_alloc_dealloc() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    assert!(unbounded_alloc::Allocator::is_initialized());

    let slot = unbounded_alloc::BoxSlot::<Order>::new(Order::new(1, 100.0));
    assert_eq!(slot.id, 1);
    assert_eq!(slot.price, 100.0);
    drop(slot);
}

#[test]
fn unbounded_multiple_types_same_slab() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let order = unbounded_alloc::BoxSlot::<Order>::new(Order::new(1, 100.0));
    let cancel = unbounded_alloc::BoxSlot::<Cancel>::new(Cancel { id: 2 });
    let num = unbounded_alloc::BoxSlot::<u64>::new(42u64);

    assert_eq!(order.id, 1);
    assert_eq!(cancel.id, 2);
    assert_eq!(*num, 42);

    drop(order);
    drop(cancel);
    drop(num);
}

#[test]
fn unbounded_deref_mut() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let mut slot = unbounded_alloc::BoxSlot::<Order>::new(Order::new(1, 100.0));
    slot.price = 200.0;
    assert_eq!(slot.price, 200.0);
    drop(slot);
}

#[test]
fn unbounded_into_inner() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let slot = unbounded_alloc::BoxSlot::<Order>::new(Order::new(42, 99.99));
    let order = slot.into_inner();
    assert_eq!(order.id, 42);
    assert_eq!(order.price, 99.99);
}

#[test]
fn unbounded_replace() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let mut slot = unbounded_alloc::BoxSlot::<Order>::new(Order::new(1, 100.0));
    let old = slot.replace(Order::new(2, 200.0));
    assert_eq!(old, Order::new(1, 100.0));
    assert_eq!(slot.id, 2);
    drop(slot);
}

#[test]
fn unbounded_leak_returns_local_static() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let slot = unbounded_alloc::BoxSlot::<Order>::new(Order::new(42, 99.99));
    let leaked: nexus_slab::LocalStatic<Order> = slot.leak();

    assert_eq!(leaked.id, 42);
    assert_eq!(leaked.price, 99.99);

    let leaked2 = leaked;
    assert_eq!(leaked2.id, 42);
}

#[test]
fn unbounded_grows_automatically() {
    mod local_alloc {
        nexus_slab::unbounded_byte_allocator!(64);
    }

    local_alloc::Allocator::builder()
        .chunk_size(4)
        .build()
        .expect("init should succeed");

    let slots: Vec<_> = (0..10)
        .map(|i| local_alloc::BoxSlot::<Order>::new(Order::new(i, i as f64)))
        .collect();

    assert!(local_alloc::Allocator::capacity() >= 10);
    drop(slots);
}

#[test]
fn unbounded_already_initialized_error() {
    mod local_alloc {
        nexus_slab::unbounded_byte_allocator!(64);
    }

    local_alloc::Allocator::builder()
        .build()
        .expect("first init should succeed");

    let result = local_alloc::Allocator::builder().build();
    assert!(result.is_err());
}

#[test]
fn unbounded_borrow_traits() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let mut slot = unbounded_alloc::BoxSlot::<Order>::new(Order::new(1, 100.0));

    let borrowed: &Order = slot.borrow();
    assert_eq!(borrowed.id, 1);

    let borrowed_mut: &mut Order = slot.borrow_mut();
    borrowed_mut.price = 200.0;
    assert_eq!(slot.price, 200.0);

    let as_ref: &Order = slot.as_ref();
    assert_eq!(as_ref.id, 1);

    drop(slot);
}

#[test]
fn unbounded_zst() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let slot = unbounded_alloc::BoxSlot::<ZeroSized>::new(ZeroSized);
    assert_eq!(*slot, ZeroSized);
    drop(slot);
}

#[test]
fn unbounded_string_type() {
    unbounded_alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    let slot = unbounded_alloc::BoxSlot::<String>::new("hello world".to_string());
    assert_eq!(&*slot, "hello world");
    drop(slot);
}

// =============================================================================
// Unbounded: Drop tracking
// =============================================================================

mod unbounded_drop_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[allow(dead_code)]
    pub struct Tracker(pub u64);
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
        nexus_slab::unbounded_byte_allocator!(64);
    }
}

#[test]
fn unbounded_drop_called() {
    unbounded_drop_test::reset();
    unbounded_drop_test::alloc::Allocator::builder()
        .build()
        .expect("init should succeed");

    {
        let _slot = unbounded_drop_test::alloc::BoxSlot::<unbounded_drop_test::Tracker>::new(
            unbounded_drop_test::Tracker(1),
        );
        assert_eq!(unbounded_drop_test::count(), 0);
    }

    assert_eq!(unbounded_drop_test::count(), 1);
}

// =============================================================================
// Trait assertion tests
// =============================================================================

fn assert_bounded_byte_alloc<A: nexus_slab::BoundedByteAlloc>() {}
fn assert_unbounded_byte_alloc<A: nexus_slab::UnboundedByteAlloc>() {}
fn assert_slab_allocator<A: nexus_slab::Alloc>() {}

#[test]
fn test_bounded_byte_trait_marker() {
    assert_bounded_byte_alloc::<bounded_alloc::Allocator>();
    assert_slab_allocator::<bounded_alloc::Allocator>();
}

#[test]
fn test_unbounded_byte_trait_marker() {
    assert_unbounded_byte_alloc::<unbounded_alloc::Allocator>();
    assert_slab_allocator::<unbounded_alloc::Allocator>();
}

#[test]
fn test_byte_box_slot_size_is_8_bytes() {
    assert_eq!(
        std::mem::size_of::<bounded_alloc::BoxSlot<Order>>(),
        8,
        "BoxSlot<Sized> should be 8 bytes (thin pointer)"
    );
    assert_eq!(
        std::mem::size_of::<unbounded_alloc::BoxSlot<Order>>(),
        8,
        "BoxSlot<Sized> should be 8 bytes (thin pointer)"
    );
}

// =============================================================================
// dyn Trait support — shared infrastructure
// =============================================================================

trait Animal: fmt::Debug {
    fn speak(&self) -> &str;
}

use std::fmt;

#[derive(Debug)]
struct Dog(String);
impl Animal for Dog {
    fn speak(&self) -> &str {
        "woof"
    }
}

#[derive(Debug)]
struct Cat;
impl Animal for Cat {
    fn speak(&self) -> &str {
        "meow"
    }
}

// =============================================================================
// Bounded dyn Trait tests
// =============================================================================

mod bounded_dyn_alloc {
    nexus_slab::bounded_byte_allocator!(64);
}

#[test]
fn bounded_dyn_unsize() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let sized = bounded_dyn_alloc::BoxSlot::<Dog>::try_new(Dog("Rex".into())).unwrap();
    let dyn_slot: bounded_dyn_alloc::BoxSlot<dyn Animal> = sized.unsize(|p| p as *mut dyn Animal);

    assert_eq!(dyn_slot.speak(), "woof");
    assert_eq!(format!("{:?}", &*dyn_slot), "Dog(\"Rex\")");
    drop(dyn_slot);
}

#[test]
fn bounded_dyn_try_new_as() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let dyn_slot = nexus_slab::byte::BoxSlot::<Dog, bounded_dyn_alloc::Allocator>::try_new_as(
        Dog("Buddy".into()),
        |p| p as *mut dyn Animal,
    )
    .unwrap();

    assert_eq!(dyn_slot.speak(), "woof");
    drop(dyn_slot);
}

#[test]
fn bounded_dyn_macro() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let dyn_slot = nexus_slab::try_box_dyn!(bounded_dyn_alloc::Allocator, dyn Animal, Cat).unwrap();

    assert_eq!(dyn_slot.speak(), "meow");
    drop(dyn_slot);
}

mod bounded_dyn_drop_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    pub struct DropDog(pub String);

    impl super::Animal for DropDog {
        fn speak(&self) -> &str {
            "woof"
        }
    }

    impl Drop for DropDog {
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
        nexus_slab::bounded_byte_allocator!(64);
    }
}

#[test]
fn bounded_dyn_drop() {
    bounded_dyn_drop_test::reset();
    bounded_dyn_drop_test::alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    {
        let dyn_slot: bounded_dyn_drop_test::alloc::BoxSlot<dyn Animal> =
            bounded_dyn_drop_test::alloc::BoxSlot::<bounded_dyn_drop_test::DropDog>::try_new(
                bounded_dyn_drop_test::DropDog("Rex".into()),
            )
            .unwrap()
            .unsize(|p| p as *mut dyn Animal);

        assert_eq!(bounded_dyn_drop_test::count(), 0);
        assert_eq!(dyn_slot.speak(), "woof");
    }

    assert_eq!(bounded_dyn_drop_test::count(), 1);
}

#[test]
fn bounded_dyn_mixed_types() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let dog: bounded_dyn_alloc::BoxSlot<dyn Animal> =
        bounded_dyn_alloc::BoxSlot::<Dog>::try_new(Dog("Rex".into()))
            .unwrap()
            .unsize(|p| p as *mut dyn Animal);

    let cat: bounded_dyn_alloc::BoxSlot<dyn Animal> =
        bounded_dyn_alloc::BoxSlot::<Cat>::try_new(Cat)
            .unwrap()
            .unsize(|p| p as *mut dyn Animal);

    assert_eq!(dog.speak(), "woof");
    assert_eq!(cat.speak(), "meow");

    drop(dog);
    drop(cat);
}

#[test]
fn bounded_dyn_leak() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let dyn_slot: bounded_dyn_alloc::BoxSlot<dyn Animal> =
        bounded_dyn_alloc::BoxSlot::<Cat>::try_new(Cat)
            .unwrap()
            .unsize(|p| p as *mut dyn Animal);

    let leaked: nexus_slab::LocalStatic<dyn Animal> = dyn_slot.leak();
    assert_eq!(leaked.speak(), "meow");

    // LocalStatic<dyn Trait> is Copy
    let leaked2 = leaked;
    assert_eq!(leaked2.speak(), "meow");
}

#[test]
fn bounded_dyn_pin() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let dyn_slot: bounded_dyn_alloc::BoxSlot<dyn Animal> =
        bounded_dyn_alloc::BoxSlot::<Dog>::try_new(Dog("Pin".into()))
            .unwrap()
            .unsize(|p| p as *mut dyn Animal);

    let addr = &raw const *dyn_slot;
    let pinned = dyn_slot.pin();
    assert_eq!(&raw const *pinned, addr);
    assert_eq!(pinned.speak(), "woof");

    drop(dyn_slot);
}

#[test]
fn bounded_dyn_size() {
    // Sized types: 8 bytes (thin pointer)
    assert_eq!(
        std::mem::size_of::<bounded_dyn_alloc::BoxSlot<Dog>>(),
        8,
        "BoxSlot<Sized> should be 8 bytes"
    );
    // dyn Trait types: 16 bytes (fat pointer = data ptr + vtable ptr)
    assert_eq!(
        std::mem::size_of::<bounded_dyn_alloc::BoxSlot<dyn Animal>>(),
        16,
        "BoxSlot<dyn Trait> should be 16 bytes"
    );
}

// =============================================================================
// Unbounded dyn Trait tests
// =============================================================================

mod unbounded_dyn_alloc {
    nexus_slab::unbounded_byte_allocator!(64);
}

#[test]
fn unbounded_dyn_unsize() {
    unbounded_dyn_alloc::Allocator::builder()
        .build()
        .expect("init");

    let sized = unbounded_dyn_alloc::BoxSlot::<Dog>::new(Dog("Rex".into()));
    let dyn_slot: unbounded_dyn_alloc::BoxSlot<dyn Animal> = sized.unsize(|p| p as *mut dyn Animal);

    assert_eq!(dyn_slot.speak(), "woof");
    drop(dyn_slot);
}

#[test]
fn unbounded_dyn_new_as() {
    unbounded_dyn_alloc::Allocator::builder()
        .build()
        .expect("init");

    let dyn_slot = nexus_slab::byte::BoxSlot::<Dog, unbounded_dyn_alloc::Allocator>::new_as(
        Dog("Buddy".into()),
        |p| p as *mut dyn Animal,
    );

    assert_eq!(dyn_slot.speak(), "woof");
    drop(dyn_slot);
}

#[test]
fn unbounded_dyn_macro() {
    unbounded_dyn_alloc::Allocator::builder()
        .build()
        .expect("init");

    let dyn_slot = nexus_slab::box_dyn!(unbounded_dyn_alloc::Allocator, dyn Animal, Cat);

    assert_eq!(dyn_slot.speak(), "meow");
    drop(dyn_slot);
}

mod unbounded_dyn_drop_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    pub struct DropCat;

    impl super::Animal for DropCat {
        fn speak(&self) -> &str {
            "meow"
        }
    }

    impl Drop for DropCat {
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
        nexus_slab::unbounded_byte_allocator!(64);
    }
}

#[test]
fn unbounded_dyn_drop() {
    unbounded_dyn_drop_test::reset();
    unbounded_dyn_drop_test::alloc::Allocator::builder()
        .build()
        .expect("init");

    {
        let dyn_slot: unbounded_dyn_drop_test::alloc::BoxSlot<dyn Animal> =
            unbounded_dyn_drop_test::alloc::BoxSlot::<unbounded_dyn_drop_test::DropCat>::new(
                unbounded_dyn_drop_test::DropCat,
            )
            .unsize(|p| p as *mut dyn Animal);

        assert_eq!(unbounded_dyn_drop_test::count(), 0);
        assert_eq!(dyn_slot.speak(), "meow");
    }

    assert_eq!(unbounded_dyn_drop_test::count(), 1);
}

#[test]
fn unbounded_dyn_mixed_types() {
    unbounded_dyn_alloc::Allocator::builder()
        .build()
        .expect("init");

    let dog: unbounded_dyn_alloc::BoxSlot<dyn Animal> =
        unbounded_dyn_alloc::BoxSlot::<Dog>::new(Dog("Rex".into()))
            .unsize(|p| p as *mut dyn Animal);

    let cat: unbounded_dyn_alloc::BoxSlot<dyn Animal> =
        unbounded_dyn_alloc::BoxSlot::<Cat>::new(Cat).unsize(|p| p as *mut dyn Animal);

    assert_eq!(dog.speak(), "woof");
    assert_eq!(cat.speak(), "meow");

    drop(dog);
    drop(cat);
}

// =============================================================================
// byte::Slot tests — bounded
// =============================================================================

#[test]
fn byte_slot_basic() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(10);

    let slot = slab.try_insert(Order::new(42, 99.99)).unwrap();
    assert_eq!(slot.id, 42);
    assert_eq!(slot.price, 99.99);

    // SAFETY: slot was allocated from this slab
    unsafe { slab.remove(slot) };
}

#[test]
fn byte_slot_dyn() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(10);

    let concrete = slab.try_insert(Dog("Fido".into())).unwrap();
    let dyn_slot: nexus_slab::byte::Slot<dyn Animal> = concrete.unsize(|p| p as *mut dyn Animal);

    assert_eq!(dyn_slot.speak(), "woof");

    // SAFETY: slot was allocated from this slab
    unsafe { slab.remove(dyn_slot) };
}

#[test]
fn byte_slot_dyn_mixed() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(10);

    let dog: nexus_slab::byte::Slot<dyn Animal> = slab
        .try_insert(Dog("Rex".into()))
        .unwrap()
        .unsize(|p| p as *mut dyn Animal);
    let cat: nexus_slab::byte::Slot<dyn Animal> = slab
        .try_insert(Cat)
        .unwrap()
        .unsize(|p| p as *mut dyn Animal);

    assert_eq!(dog.speak(), "woof");
    assert_eq!(cat.speak(), "meow");

    // SAFETY: slots were allocated from this slab
    unsafe {
        slab.remove(dog);
        slab.remove(cat);
    }
}

mod slot_box_drop_test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNT: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    pub struct DropAnimal(pub &'static str);

    impl super::Animal for DropAnimal {
        fn speak(&self) -> &str {
            self.0
        }
    }

    impl Drop for DropAnimal {
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
}

#[test]
fn byte_slot_drop_count() {
    slot_box_drop_test::reset();

    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(10);

    let slot = slab
        .try_insert(slot_box_drop_test::DropAnimal("hello"))
        .unwrap();
    let dyn_slot: nexus_slab::byte::Slot<dyn Animal> = slot.unsize(|p| p as *mut dyn Animal);

    assert_eq!(slot_box_drop_test::count(), 0);
    // SAFETY: slot was allocated from this slab
    unsafe { slab.remove(dyn_slot) };
    assert_eq!(slot_box_drop_test::count(), 1);
}

#[test]
fn byte_slot_take_value() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(10);

    let slot = slab.try_insert(Order::new(1, 1.0)).unwrap();

    // SAFETY: slot was allocated from this slab
    let value = unsafe { slab.take_value(slot) };
    assert_eq!(value.id, 1);
    assert_eq!(value.price, 1.0);
}

#[test]
fn byte_slot_reclaim() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(10);

    let slot = slab.try_insert(Order::new(1, 1.0)).unwrap();

    // Read the value out manually via Deref, then move out with ptr::read
    let value = unsafe { std::ptr::read(&*slot as *const Order) };
    assert_eq!(value.id, 1);

    // SAFETY: slot was allocated from this slab, value moved out
    unsafe { slab.reclaim(slot) };
}

#[test]
fn byte_slot_size() {
    assert_eq!(
        std::mem::size_of::<nexus_slab::byte::Slot<Order>>(),
        8,
        "Slot<Sized> should be 8 bytes (thin pointer)"
    );
    assert_eq!(
        std::mem::size_of::<nexus_slab::byte::Slot<dyn Animal>>(),
        16,
        "Slot<dyn Trait> should be 16 bytes (fat pointer)"
    );
}

// =============================================================================
// byte::Slot tests — unbounded
// =============================================================================

#[test]
fn byte_slot_unbounded_basic() {
    let slab = nexus_slab::unbounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(16);

    let slot = slab.insert(Order::new(7, 77.77));
    assert_eq!(slot.id, 7);
    assert_eq!(slot.price, 77.77);

    // SAFETY: slot was allocated from this slab
    unsafe { slab.remove(slot) };
}

#[test]
fn byte_slot_unbounded_dyn() {
    let slab = nexus_slab::unbounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(16);

    let concrete = slab.insert(Cat);
    let dyn_slot: nexus_slab::byte::Slot<dyn Animal> = concrete.unsize(|p| p as *mut dyn Animal);

    assert_eq!(dyn_slot.speak(), "meow");

    // SAFETY: slot was allocated from this slab
    unsafe { slab.remove(dyn_slot) };
}

// =============================================================================
// SlotBox debug-mode leak detection
// =============================================================================

#[cfg(debug_assertions)]
#[test]
fn byte_slot_debug_leak() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(10);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _slot = slab.try_insert(42u64).unwrap();
        // slot drops here without being freed
    }));

    assert!(result.is_err(), "Slot should panic on drop in debug mode");
}

// =============================================================================
// Additional dyn Trait test gaps
// =============================================================================

#[test]
fn dyn_debug_format() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let dyn_slot: bounded_dyn_alloc::BoxSlot<dyn Animal> =
        bounded_dyn_alloc::BoxSlot::<Dog>::try_new(Dog("Rex".into()))
            .unwrap()
            .unsize(|p| p as *mut dyn Animal);

    let debug = format!("{:?}", dyn_slot);
    assert!(debug.contains("BoxSlot"));
    assert!(debug.contains("Dog"));
    assert!(debug.contains("Rex"));

    drop(dyn_slot);
}

#[test]
fn dyn_borrow_traits() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let mut dyn_slot: bounded_dyn_alloc::BoxSlot<dyn Animal> =
        bounded_dyn_alloc::BoxSlot::<Cat>::try_new(Cat)
            .unwrap()
            .unsize(|p| p as *mut dyn Animal);

    let borrowed: &dyn Animal = dyn_slot.borrow();
    assert_eq!(borrowed.speak(), "meow");

    let borrowed_mut: &mut dyn Animal = dyn_slot.borrow_mut();
    assert_eq!(borrowed_mut.speak(), "meow");

    let as_ref: &dyn Animal = dyn_slot.as_ref();
    assert_eq!(as_ref.speak(), "meow");

    drop(dyn_slot);
}

#[test]
fn dyn_pin_mut() {
    bounded_dyn_alloc::Allocator::builder()
        .capacity(10)
        .build()
        .expect("init");

    let mut dyn_slot: bounded_dyn_alloc::BoxSlot<dyn Animal> =
        bounded_dyn_alloc::BoxSlot::<Dog>::try_new(Dog("Pinned".into()))
            .unwrap()
            .unsize(|p| p as *mut dyn Animal);

    let addr = &raw const *dyn_slot;
    let pinned = dyn_slot.pin_mut();
    assert_eq!(&raw const *pinned, addr);
    assert_eq!(pinned.speak(), "woof");

    drop(dyn_slot);
}

#[test]
fn unbounded_dyn_size() {
    assert_eq!(
        std::mem::size_of::<unbounded_dyn_alloc::BoxSlot<Dog>>(),
        8,
        "BoxSlot<Sized> should be 8 bytes"
    );
    assert_eq!(
        std::mem::size_of::<unbounded_dyn_alloc::BoxSlot<dyn Animal>>(),
        16,
        "BoxSlot<dyn Trait> should be 16 bytes"
    );
}
