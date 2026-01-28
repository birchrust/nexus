//! Drop verification tests.
//!
//! These tests verify that values are properly dropped when:
//! - Entry::remove() is called
//! - Slab is dropped
//! - clear() is called
//! - VacantEntry is dropped without inserting

use std::cell::Cell;
use std::rc::Rc;

use nexus_slab::{BoundedSlab, Slab};

/// A value that tracks whether it was dropped.
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

// =============================================================================
// BoundedSlab Drop tests
// =============================================================================

#[test]
fn bounded_remove_drops_value() {
    let counter = Rc::new(Cell::new(0));

    let slab = BoundedSlab::with_capacity(16);
    let entry = slab.insert(DropTracker::new(counter.clone())).unwrap();

    assert_eq!(counter.get(), 0);
    entry.remove();
    assert_eq!(counter.get(), 1);
}

#[test]
fn bounded_slab_drop_drops_all_values() {
    let counter = Rc::new(Cell::new(0));

    {
        let slab = BoundedSlab::with_capacity(16);
        for _ in 0..10 {
            slab.insert(DropTracker::new(counter.clone())).unwrap();
        }
        assert_eq!(counter.get(), 0);
    }

    assert_eq!(counter.get(), 10);
}

#[test]
fn bounded_clear_drops_all_values() {
    let counter = Rc::new(Cell::new(0));

    let slab = BoundedSlab::with_capacity(16);
    for _ in 0..10 {
        slab.insert(DropTracker::new(counter.clone())).unwrap();
    }

    assert_eq!(counter.get(), 0);
    slab.clear();
    assert_eq!(counter.get(), 10);

    // Slab is now empty, can reuse
    assert!(slab.is_empty());
}

#[test]
fn bounded_replace_drops_old_value() {
    let counter = Rc::new(Cell::new(0));

    let slab = BoundedSlab::with_capacity(16);
    let entry = slab.insert(DropTracker::new(counter.clone())).unwrap();

    assert_eq!(counter.get(), 0);

    // Replace drops the returned value when we drop it
    let old = entry.replace(DropTracker::new(counter.clone()));
    drop(old);
    assert_eq!(counter.get(), 1);

    // Original entry now holds new tracker
    entry.remove();
    assert_eq!(counter.get(), 2);
}

#[test]
fn bounded_take_returns_value_without_dropping() {
    let counter = Rc::new(Cell::new(0));

    let slab = BoundedSlab::with_capacity(16);
    let entry = slab.insert(DropTracker::new(counter.clone())).unwrap();

    assert_eq!(counter.get(), 0);

    let (value, vacant) = entry.take();
    // Value extracted, not dropped yet
    assert_eq!(counter.get(), 0);

    drop(value);
    assert_eq!(counter.get(), 1);

    // VacantEntry drop doesn't drop anything (no value)
    drop(vacant);
    assert_eq!(counter.get(), 1);
}

#[test]
fn bounded_vacant_entry_drop_no_drop() {
    let counter = Rc::new(Cell::new(0));

    let slab: BoundedSlab<DropTracker> = BoundedSlab::with_capacity(16);
    {
        let _vacant = slab.vacant_entry().unwrap();
        // Don't insert anything
    }

    // Nothing should be dropped
    assert_eq!(counter.get(), 0);
    assert!(slab.is_empty());
}

#[test]
fn bounded_partial_fill_drops_only_occupied() {
    let counter = Rc::new(Cell::new(0));

    {
        let slab = BoundedSlab::with_capacity(16);

        // Insert 5, remove 2
        let e1 = slab.insert(DropTracker::new(counter.clone())).unwrap();
        let _e2 = slab.insert(DropTracker::new(counter.clone())).unwrap();
        let e3 = slab.insert(DropTracker::new(counter.clone())).unwrap();
        let _e4 = slab.insert(DropTracker::new(counter.clone())).unwrap();
        let _e5 = slab.insert(DropTracker::new(counter.clone())).unwrap();

        // Remove 2 of them
        e1.remove();
        e3.remove();
        assert_eq!(counter.get(), 2);

        // Slab drop should drop remaining 3
    }

    assert_eq!(counter.get(), 5);
}

// =============================================================================
// Slab (unbounded) Drop tests
// =============================================================================

#[test]
fn unbounded_remove_drops_value() {
    let counter = Rc::new(Cell::new(0));

    let slab = Slab::with_capacity(16);
    let entry = slab.insert(DropTracker::new(counter.clone()));

    assert_eq!(counter.get(), 0);
    entry.remove();
    assert_eq!(counter.get(), 1);
}

#[test]
fn unbounded_slab_drop_drops_all_values() {
    let counter = Rc::new(Cell::new(0));

    {
        let slab = Slab::with_capacity(16);
        for _ in 0..10 {
            slab.insert(DropTracker::new(counter.clone()));
        }
        assert_eq!(counter.get(), 0);
    }

    assert_eq!(counter.get(), 10);
}

#[test]
fn unbounded_clear_drops_all_values() {
    let counter = Rc::new(Cell::new(0));

    let slab = Slab::with_capacity(16);
    for _ in 0..10 {
        slab.insert(DropTracker::new(counter.clone()));
    }

    assert_eq!(counter.get(), 0);
    slab.clear();
    assert_eq!(counter.get(), 10);

    assert!(slab.is_empty());
}

#[test]
fn unbounded_multi_chunk_drops_all() {
    let counter = Rc::new(Cell::new(0));

    {
        // Small chunk size to force multiple chunks
        let slab: Slab<DropTracker> = Slab::builder().chunk_capacity(4).build();

        // Insert enough to span multiple chunks
        for _ in 0..20 {
            slab.insert(DropTracker::new(counter.clone()));
        }

        assert_eq!(counter.get(), 0);
    }

    assert_eq!(counter.get(), 20);
}

#[test]
fn unbounded_replace_drops_old_value() {
    let counter = Rc::new(Cell::new(0));

    let slab = Slab::with_capacity(16);
    let entry = slab.insert(DropTracker::new(counter.clone()));

    assert_eq!(counter.get(), 0);

    let old = entry.replace(DropTracker::new(counter.clone()));
    drop(old);
    assert_eq!(counter.get(), 1);

    entry.remove();
    assert_eq!(counter.get(), 2);
}

#[test]
fn unbounded_take_returns_value_without_dropping() {
    let counter = Rc::new(Cell::new(0));

    let slab = Slab::with_capacity(16);
    let entry = slab.insert(DropTracker::new(counter.clone()));

    assert_eq!(counter.get(), 0);

    let (value, vacant) = entry.take();
    assert_eq!(counter.get(), 0);

    drop(value);
    assert_eq!(counter.get(), 1);

    drop(vacant);
    assert_eq!(counter.get(), 1);
}

#[test]
fn unbounded_vacant_entry_drop_no_drop() {
    let counter = Rc::new(Cell::new(0));

    let slab: Slab<DropTracker> = Slab::with_capacity(16);
    {
        let _vacant = slab.vacant_entry();
    }

    assert_eq!(counter.get(), 0);
    assert!(slab.is_empty());
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn bounded_remove_by_key_drops_value() {
    let counter = Rc::new(Cell::new(0));

    let slab = BoundedSlab::with_capacity(16);
    let key = slab.insert(DropTracker::new(counter.clone())).unwrap().key();

    assert_eq!(counter.get(), 0);
    let removed = slab.remove_by_key(key);
    drop(removed);
    assert_eq!(counter.get(), 1);
}

#[test]
fn unbounded_remove_by_key_drops_value() {
    let counter = Rc::new(Cell::new(0));

    let slab = Slab::with_capacity(16);
    let key = slab.insert(DropTracker::new(counter.clone())).key();

    assert_eq!(counter.get(), 0);
    let removed = slab.remove_by_key(key);
    drop(removed);
    assert_eq!(counter.get(), 1);
}

#[test]
fn bounded_insert_with_closure_drops_on_remove() {
    let counter = Rc::new(Cell::new(0));
    let counter_clone = counter.clone();

    let slab = BoundedSlab::with_capacity(16);
    let entry = slab.insert_with(|_| DropTracker::new(counter_clone)).unwrap();

    assert_eq!(counter.get(), 0);
    entry.remove();
    assert_eq!(counter.get(), 1);
}

#[test]
fn unbounded_insert_with_closure_drops_on_remove() {
    let counter = Rc::new(Cell::new(0));
    let counter_clone = counter.clone();

    let slab = Slab::with_capacity(16);
    let entry = slab.insert_with(|_| DropTracker::new(counter_clone));

    assert_eq!(counter.get(), 0);
    entry.remove();
    assert_eq!(counter.get(), 1);
}
