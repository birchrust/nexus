//! Tests designed to be run under Miri for undefined behavior detection.
//!
//! Run with: `MIRIFLAGS="-Zmiri-ignore-leaks" cargo +nightly miri test --test miri_tests`
//!
//! These tests focus on:
//! - Unsafe key-based API (get_by_key, get_by_key_mut, remove_by_key)
//! - Drop correctness (use-after-free, double-free)
//! - Pointer validity (dangling pointers, aliasing)
//!
//! Note: We use -Zmiri-ignore-leaks because the slab allocator intentionally
//! leaks memory (static allocator pattern).

use std::cell::Cell;
use std::rc::Rc;

use nexus_slab::{Key, bounded, unbounded};

// =============================================================================
// BoundedSlab unsafe paths
// =============================================================================

#[test]
fn bounded_insert_unchecked() {
    let slab: bounded::Slab<u64> = bounded::Slab::with_capacity(16);

    for i in 0..16u64 {
        let entry = unsafe { slab.insert_unchecked(i) };
        assert_eq!(*entry.get(), i);
        entry.forget(); // Keep alive
    }

    // Verify all values via unsafe key access
    for i in 0..16u64 {
        let key = Key::from_raw(i as u32);
        // SAFETY: key is valid (we just inserted it)
        assert_eq!(unsafe { *slab.get_by_key(key) }, i);
    }
}

#[test]
fn bounded_get_by_key() {
    let slab: bounded::Slab<u64> = bounded::Slab::with_capacity(16);
    let entry = slab.try_insert(42u64).unwrap();
    let key = entry.forget();

    // SAFETY: key is valid
    unsafe {
        assert_eq!(*slab.get_by_key(key), 42);
        *slab.get_by_key_mut(key) = 100;
        assert_eq!(*slab.get_by_key(key), 100);
    }

    // Clean up
    // SAFETY: key is valid
    unsafe { slab.remove_by_key(key) };
}

#[test]
fn bounded_remove_by_key() {
    let slab: bounded::Slab<u64> = bounded::Slab::with_capacity(16);

    let keys: Vec<Key> = (0..16u64)
        .map(|i| slab.try_insert(i).unwrap().forget())
        .collect();

    for (i, key) in keys.iter().enumerate() {
        // SAFETY: key is valid
        let value = unsafe { slab.remove_by_key(*key) };
        assert_eq!(value, i as u64);
    }

    assert!(slab.is_empty());
}

#[test]
fn bounded_entry_take() {
    let slab: bounded::Slab<u64> = bounded::Slab::with_capacity(16);
    let entry = slab.try_insert(42u64).unwrap();
    let key = entry.key();

    let (value, vacant) = entry.take();
    assert_eq!(value, 42);
    assert_eq!(vacant.key(), key);

    // Re-insert
    let new_entry = vacant.insert(100);
    assert_eq!(new_entry.key(), key);
    assert_eq!(*new_entry.get(), 100);
}

// =============================================================================
// Slab (unbounded) unsafe paths
// =============================================================================

#[test]
fn unbounded_get_by_key() {
    let slab = unbounded::Slab::with_capacity(16);
    let entry = slab.insert(42u64);
    let key = entry.forget();

    // SAFETY: key is valid
    unsafe {
        assert_eq!(*slab.get_by_key(key), 42);
        *slab.get_by_key_mut(key) = 100;
        assert_eq!(*slab.get_by_key(key), 100);
    }

    // Clean up
    // SAFETY: key is valid
    unsafe { slab.remove_by_key(key) };
}

#[test]
fn unbounded_remove_by_key() {
    let slab: unbounded::Slab<u64> = nexus_slab::Builder::default()
        .unbounded()
        .chunk_capacity(4)
        .build();

    // Insert across multiple chunks
    let keys: Vec<Key> = (0..16u64).map(|i| slab.insert(i).forget()).collect();

    for (i, key) in keys.iter().enumerate() {
        // SAFETY: key is valid
        let value = unsafe { slab.remove_by_key(*key) };
        assert_eq!(value, i as u64);
    }

    assert!(slab.is_empty());
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

    let slab: bounded::Slab<DropTracker> = bounded::Slab::with_capacity(10);
    for _ in 0..10 {
        unsafe {
            slab.insert_unchecked(DropTracker::new(counter.clone()))
                .forget();
        }
    }

    assert_eq!(counter.get(), 0);
    slab.clear();
    assert_eq!(counter.get(), 10);
}

#[test]
fn bounded_drop_on_remove_by_key() {
    let counter = Rc::new(Cell::new(0));

    let slab: bounded::Slab<DropTracker> = bounded::Slab::with_capacity(10);
    let keys: Vec<Key> = (0..10)
        .map(|_| unsafe {
            slab.insert_unchecked(DropTracker::new(counter.clone()))
                .forget()
        })
        .collect();

    for key in keys {
        // SAFETY: key is valid
        let tracker = unsafe { slab.remove_by_key(key) };
        drop(tracker);
    }

    assert_eq!(counter.get(), 10);
}

#[test]
fn unbounded_drop_on_clear_multi_chunk() {
    let counter = Rc::new(Cell::new(0));

    let slab: unbounded::Slab<DropTracker> = nexus_slab::Builder::default()
        .unbounded()
        .chunk_capacity(4)
        .build();
    for _ in 0..20 {
        slab.insert(DropTracker::new(counter.clone())).forget();
    }

    assert_eq!(counter.get(), 0);
    slab.clear();
    assert_eq!(counter.get(), 20);
}

// =============================================================================
// Pointer validity tests
// =============================================================================

#[test]
fn bounded_entry_valid_after_other_removes() {
    let slab: bounded::Slab<u64> = bounded::Slab::with_capacity(16);

    // Insert multiple entries
    let e1 = slab.try_insert(1u64).unwrap();
    let e2 = slab.try_insert(2u64).unwrap();
    let e3 = slab.try_insert(3u64).unwrap();

    // Remove middle entry
    e2.remove();

    // Other entries should still be valid and accessible
    assert_eq!(*e1.get(), 1);
    assert_eq!(*e3.get(), 3);
}

#[test]
fn unbounded_key_valid_after_other_removes() {
    let slab: unbounded::Slab<u64> = nexus_slab::Builder::default()
        .unbounded()
        .chunk_capacity(4)
        .build();

    // Insert across multiple chunks, forget all to keep them alive
    let keys: Vec<Key> = (0..12u64).map(|i| slab.insert(i).forget()).collect();

    // Remove some via key
    // SAFETY: keys are valid
    unsafe {
        slab.remove_by_key(keys[1]);
        slab.remove_by_key(keys[5]);
        slab.remove_by_key(keys[9]);
    }

    // Remaining should be valid
    for (i, key) in keys.iter().enumerate() {
        if i != 1 && i != 5 && i != 9 {
            // SAFETY: key is valid
            unsafe {
                assert_eq!(*slab.get_by_key(*key), i as u64);
            }
        }
    }
}

#[test]
fn bounded_slot_reuse_no_use_after_free() {
    let counter = Rc::new(Cell::new(0));
    let slab: bounded::Slab<DropTracker> = bounded::Slab::with_capacity(1);

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
    let slab: unbounded::Slab<DropTracker> = nexus_slab::Builder::default()
        .unbounded()
        .chunk_capacity(1)
        .build();

    for i in 0..100u64 {
        let entry = slab.insert(DropTracker::new(counter.clone()));
        entry.remove();
        assert_eq!(counter.get(), (i + 1) as usize);
    }
}

// =============================================================================
// Entry mutability tests
// =============================================================================

#[test]
fn bounded_entry_get_mut() {
    let slab: bounded::Slab<u64> = bounded::Slab::with_capacity(16);

    let mut e1 = slab.try_insert(1u64).unwrap();
    let mut e2 = slab.try_insert(2u64).unwrap();

    // Mutate through entry
    *e1.get_mut() = 10;
    *e2.get_mut() = 20;

    assert_eq!(*e1.get(), 10);
    assert_eq!(*e2.get(), 20);
}

#[test]
fn unbounded_entry_get_mut() {
    let slab = unbounded::Slab::with_capacity(16);

    let mut e1 = slab.insert(1u64);
    let mut e2 = slab.insert(2u64);

    *e1.get_mut() = 10;
    *e2.get_mut() = 20;

    assert_eq!(*e1.get(), 10);
    assert_eq!(*e2.get(), 20);
}

#[test]
fn unbounded_entry_get_mut_across_chunks() {
    let slab: unbounded::Slab<u64> = nexus_slab::Builder::default()
        .unbounded()
        .chunk_capacity(2)
        .build();

    // These will be in different chunks
    let mut e1 = slab.insert(1);
    let _e2 = slab.insert(2);
    let mut e3 = slab.insert(3); // Different chunk
    let _e4 = slab.insert(4); // Different chunk

    *e1.get_mut() = 10;
    *e3.get_mut() = 30;

    assert_eq!(*e1.get(), 10);
    assert_eq!(*e3.get(), 30);
}
