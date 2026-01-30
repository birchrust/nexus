//! Tests designed to be run under Miri for undefined behavior detection.
//!
//! Run with: `MIRIFLAGS="-Zmiri-ignore-leaks" cargo +nightly miri test --test miri_tests`
//!
//! These tests focus on:
//! - Unsafe code paths (unchecked access, manual memory management)
//! - Drop correctness (use-after-free, double-free)
//! - Pointer validity (dangling pointers, aliasing)
//!
//! Note: We use -Zmiri-ignore-leaks because the slab allocator intentionally
//! leaks memory (static allocator pattern).

use std::cell::Cell;
use std::rc::Rc;

use nexus_slab::{BoundedSlab, Key, Slab};

// =============================================================================
// BoundedSlab unsafe paths
// =============================================================================

#[test]
fn bounded_insert_unchecked() {
    let slab: BoundedSlab<u64> = BoundedSlab::leak(16);

    for i in 0..16u64 {
        let entry = unsafe { slab.insert_unchecked(i) };
        assert_eq!(*entry.get(), i);
        entry.leak(); // Keep alive
    }

    // Verify all values
    for i in 0..16u64 {
        let key = Key::from_raw(i as u32);
        assert_eq!(*slab.get(key).unwrap(), i);
    }
}

#[test]
fn bounded_get_unchecked() {
    let slab: BoundedSlab<u64> = BoundedSlab::leak(16);
    let entry = slab.try_insert(42u64).unwrap();

    // Entry unchecked
    unsafe {
        assert_eq!(*entry.get_unchecked(), 42);
        *entry.get_unchecked_mut() = 100;
        assert_eq!(*entry.get_unchecked(), 100);
    }

    // Slab unchecked
    let key = entry.key();
    unsafe {
        assert_eq!(*slab.get_unchecked(key), 100);
        *slab.get_unchecked_mut(key) = 200;
        assert_eq!(*slab.get_unchecked(key), 200);
    }
}

#[test]
fn bounded_get_untracked() {
    let slab: BoundedSlab<u64> = BoundedSlab::leak(16);
    let entry = slab.try_insert(42u64).unwrap();
    let key = entry.key();

    unsafe {
        // Slab untracked
        assert_eq!(slab.get_untracked(key), Some(&42));
        assert_eq!(slab.get_untracked_mut(key), Some(&mut 42));

        // Entry untracked
        assert_eq!(entry.get_untracked(), Some(&42));
        assert_eq!(entry.get_untracked_mut(), Some(&mut 42));
    }
}

#[test]
fn bounded_remove_unchecked_by_key() {
    let slab: BoundedSlab<u64> = BoundedSlab::leak(16);

    let keys: Vec<Key> = (0..16u64)
        .map(|i| slab.try_insert(i).unwrap().leak())
        .collect();

    for (i, key) in keys.iter().enumerate() {
        let value = unsafe { slab.remove_unchecked_by_key(*key) };
        assert_eq!(value, i as u64);
    }

    assert!(slab.is_empty());
}

#[test]
fn bounded_take_unchecked() {
    let slab: BoundedSlab<u64> = BoundedSlab::leak(16);
    let entry = slab.try_insert(42u64).unwrap();
    let key = entry.key();

    let (value, vacant) = unsafe { entry.take_unchecked() };
    assert_eq!(value, 42);
    assert_eq!(vacant.key(), key);

    // Re-insert
    let new_entry = vacant.insert(100);
    assert_eq!(new_entry.key(), key);
    assert_eq!(*new_entry.get(), 100);
}

#[test]
fn bounded_untracked_accessor() {
    let slab: BoundedSlab<u64> = BoundedSlab::leak(16);
    let entry = slab.try_insert(42u64).unwrap();
    let key = entry.key();

    unsafe {
        let accessor = slab.untracked();
        assert_eq!(accessor[key], 42);
    }

    unsafe {
        let mut accessor = slab.untracked();
        accessor[key] = 100;
    }

    assert_eq!(*entry.get(), 100);
}

// =============================================================================
// Slab (unbounded) unsafe paths
// =============================================================================

#[test]
fn unbounded_get_unchecked() {
    let slab = Slab::with_capacity(16);
    let entry = slab.insert(42u64);

    unsafe {
        assert_eq!(*entry.get_unchecked(), 42);
        *entry.get_unchecked_mut() = 100;
        assert_eq!(*entry.get_unchecked(), 100);
    }

    let key = entry.key();
    unsafe {
        assert_eq!(*slab.get_unchecked(key), 100);
        *slab.get_unchecked_mut(key) = 200;
        assert_eq!(*slab.get_unchecked(key), 200);
    }
}

#[test]
fn unbounded_get_untracked() {
    let slab = Slab::with_capacity(16);
    let entry = slab.insert(42u64);
    let key = entry.key();

    unsafe {
        assert_eq!(slab.get_untracked(key), Some(&42));
        assert_eq!(slab.get_untracked_mut(key), Some(&mut 42));
        assert_eq!(entry.get_untracked(), Some(&42));
        assert_eq!(entry.get_untracked_mut(), Some(&mut 42));
    }
}

#[test]
fn unbounded_remove_unchecked_by_key() {
    let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

    // Insert across multiple chunks
    let keys: Vec<Key> = (0..16u64).map(|i| slab.insert(i).leak()).collect();

    for (i, key) in keys.iter().enumerate() {
        let value = unsafe { slab.remove_unchecked_by_key(*key) };
        assert_eq!(value, i as u64);
    }

    assert!(slab.is_empty());
}

#[test]
fn unbounded_untracked_accessor() {
    let slab = Slab::with_capacity(16);
    let entry = slab.insert(42u64);
    let key = entry.key();

    unsafe {
        let accessor = slab.untracked();
        assert_eq!(accessor[key], 42);
    }

    unsafe {
        let mut accessor = slab.untracked();
        accessor[key] = 100;
    }

    assert_eq!(*entry.get(), 100);
}

// =============================================================================
// Memory safety: Drop tracking
// =============================================================================

#[derive(Debug)]
struct DropTracker {
    counter: Rc<Cell<usize>>,
}

impl DropTracker {
    fn new(counter: Rc<Cell<usize>>) -> Self {
        Self { counter }
    }
}

impl Drop for DropTracker {
    fn drop(&mut self) {
        self.counter.set(self.counter.get() + 1);
    }
}

#[test]
fn bounded_drop_on_clear() {
    let counter = Rc::new(Cell::new(0));

    let slab: BoundedSlab<DropTracker> = BoundedSlab::leak(10);
    for _ in 0..10 {
        unsafe {
            slab.insert_unchecked(DropTracker::new(counter.clone()))
                .leak();
        }
    }

    assert_eq!(counter.get(), 0);
    slab.clear();
    assert_eq!(counter.get(), 10);
}

#[test]
fn bounded_drop_on_remove_unchecked() {
    let counter = Rc::new(Cell::new(0));

    let slab: BoundedSlab<DropTracker> = BoundedSlab::leak(10);
    let keys: Vec<Key> = (0..10)
        .map(|_| unsafe {
            slab.insert_unchecked(DropTracker::new(counter.clone()))
                .leak()
        })
        .collect();

    for key in keys {
        let tracker = unsafe { slab.remove_unchecked_by_key(key) };
        drop(tracker);
    }

    assert_eq!(counter.get(), 10);
}

#[test]
fn unbounded_drop_on_clear_multi_chunk() {
    let counter = Rc::new(Cell::new(0));

    let slab: Slab<DropTracker> = Slab::builder().chunk_capacity(4).build();
    for _ in 0..20 {
        slab.insert(DropTracker::new(counter.clone())).leak();
    }

    assert_eq!(counter.get(), 0);
    slab.clear();
    assert_eq!(counter.get(), 20);
}

// =============================================================================
// Pointer validity tests
// =============================================================================

#[test]
fn bounded_entry_pointer_valid_after_other_removes() {
    let slab: BoundedSlab<u64> = BoundedSlab::leak(16);

    // Insert multiple entries
    let e1 = slab.try_insert(1u64).unwrap();
    let e2 = slab.try_insert(2u64).unwrap();
    let e3 = slab.try_insert(3u64).unwrap();

    // Remove middle entry
    e2.remove();

    // Other entries should still be valid and accessible
    unsafe {
        assert_eq!(*e1.get_unchecked(), 1);
        assert_eq!(*e3.get_unchecked(), 3);
    }
}

#[test]
fn unbounded_entry_pointer_valid_after_other_removes() {
    let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

    // Insert across multiple chunks, leak all to keep them alive
    let keys: Vec<Key> = (0..12u64).map(|i| slab.insert(i).leak()).collect();

    // Remove some via key
    slab.remove_by_key(keys[1]);
    slab.remove_by_key(keys[5]);
    slab.remove_by_key(keys[9]);

    // Remaining should be valid
    for (i, key) in keys.iter().enumerate() {
        if i != 1 && i != 5 && i != 9 {
            unsafe {
                assert_eq!(*slab.get_unchecked(*key), i as u64);
            }
        }
    }
}

#[test]
fn bounded_slot_reuse_no_use_after_free() {
    let counter = Rc::new(Cell::new(0));
    let slab: BoundedSlab<DropTracker> = BoundedSlab::leak(1);

    // Insert, remove, insert cycle
    for i in 0..100u64 {
        let entry = slab.try_insert(DropTracker::new(counter.clone())).unwrap();
        entry.remove();
        // Each iteration should have dropped the value
        assert_eq!(counter.get(), (i + 1) as usize);
    }
}

#[test]
fn unbounded_slot_reuse_no_use_after_free() {
    let counter = Rc::new(Cell::new(0));
    let slab: Slab<DropTracker> = Slab::builder().chunk_capacity(1).build();

    for i in 0..100u64 {
        let entry = slab.insert(DropTracker::new(counter.clone()));
        entry.remove();
        assert_eq!(counter.get(), (i + 1) as usize);
    }
}

// =============================================================================
// Aliasing tests (ensure no mutable aliasing)
// =============================================================================

#[test]
fn bounded_no_aliasing_through_entries() {
    let slab: BoundedSlab<u64> = BoundedSlab::leak(16);

    let e1 = slab.try_insert(1u64).unwrap();
    let e2 = slab.try_insert(2u64).unwrap();

    // Get mutable references to different slots - should not alias
    unsafe {
        let r1 = e1.get_unchecked_mut();
        let r2 = e2.get_unchecked_mut();

        *r1 = 10;
        *r2 = 20;

        assert_eq!(*r1, 10);
        assert_eq!(*r2, 20);
    }
}

#[test]
fn unbounded_no_aliasing_through_entries() {
    let slab = Slab::with_capacity(16);

    let e1 = slab.insert(1u64);
    let e2 = slab.insert(2u64);

    unsafe {
        let r1 = e1.get_unchecked_mut();
        let r2 = e2.get_unchecked_mut();

        *r1 = 10;
        *r2 = 20;

        assert_eq!(*r1, 10);
        assert_eq!(*r2, 20);
    }
}

#[test]
fn unbounded_no_aliasing_across_chunks() {
    let slab: Slab<u64> = Slab::builder().chunk_capacity(2).build();

    // These will be in different chunks
    let e1 = slab.insert(1);
    let e2 = slab.insert(2);
    let e3 = slab.insert(3); // Different chunk
    let e4 = slab.insert(4); // Different chunk

    unsafe {
        let r1 = e1.get_unchecked_mut();
        let r3 = e3.get_unchecked_mut();

        *r1 = 10;
        *r3 = 30;

        assert_eq!(*r1, 10);
        assert_eq!(*r3, 30);
    }

    unsafe {
        let r2 = e2.get_unchecked_mut();
        let r4 = e4.get_unchecked_mut();

        *r2 = 20;
        *r4 = 40;

        assert_eq!(*r2, 20);
        assert_eq!(*r4, 40);
    }
}
