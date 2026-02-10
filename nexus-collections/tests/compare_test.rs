//! Tests for custom comparator support (Reverse ordering).

use nexus_collections::Reverse;
use serial_test::serial;

// Both tree types share the same node allocator regardless of comparator.
// The comparator only affects key ordering at the tree level.

#[allow(dead_code)]
mod rb {
    nexus_collections::rbtree_allocator!(u64, String, bounded);
}

#[allow(dead_code)]
mod bt {
    nexus_collections::btree_allocator!(u64, String, bounded);
}

fn init() {
    let _ = rb::Allocator::builder().capacity(200).build();
    let _ = bt::Allocator::builder().capacity(500).build();
}

// =============================================================================
// RbTree with Reverse
// =============================================================================

#[test]
#[serial]
fn rbtree_reverse_first_last() {
    init();
    let mut map = rb::RbTree::with_comparator(rb::Allocator, Reverse);
    map.try_insert(10, "ten".into()).unwrap();
    map.try_insert(30, "thirty".into()).unwrap();
    map.try_insert(20, "twenty".into()).unwrap();

    // Reverse: "first" (leftmost) is the largest key.
    assert_eq!(map.first_key_value(), Some((&30, &"thirty".into())));
    // Reverse: "last" (rightmost) is the smallest key.
    assert_eq!(map.last_key_value(), Some((&10, &"ten".into())));
    map.verify_invariants();
}

#[test]
#[serial]
fn rbtree_reverse_pop_first() {
    init();
    let mut map = rb::RbTree::with_comparator(rb::Allocator, Reverse);
    for k in [50, 30, 80, 10, 70, 20, 60, 40] {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    map.verify_invariants();

    // pop_first should yield keys in descending order (largest first).
    let mut popped = Vec::new();
    while let Some((k, _)) = map.pop_first() {
        popped.push(k);
        map.verify_invariants();
    }
    assert_eq!(popped, vec![80, 70, 60, 50, 40, 30, 20, 10]);
}

#[test]
#[serial]
fn rbtree_reverse_iteration_order() {
    init();
    let mut map = rb::RbTree::with_comparator(rb::Allocator, Reverse);
    for k in 1..=5 {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.iter().map(|(&k, _)| k).collect();
    assert_eq!(keys, vec![5, 4, 3, 2, 1]);
}

#[test]
#[serial]
fn rbtree_reverse_get_and_remove() {
    init();
    let mut map = rb::RbTree::with_comparator(rb::Allocator, Reverse);
    map.try_insert(10, "ten".into()).unwrap();
    map.try_insert(20, "twenty".into()).unwrap();

    assert_eq!(map.get(&10), Some(&"ten".into()));
    assert_eq!(map.get(&20), Some(&"twenty".into()));
    assert_eq!(map.get(&99), None);

    assert_eq!(map.remove(&10), Some("ten".into()));
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&10), None);
    map.verify_invariants();
}

#[test]
#[serial]
fn rbtree_reverse_entry_api() {
    init();
    let mut map = rb::RbTree::with_comparator(rb::Allocator, Reverse);

    // Vacant insert.
    map.entry(10).or_try_insert("ten".into()).unwrap();
    assert_eq!(map.get(&10), Some(&"ten".into()));

    // Occupied — should not overwrite.
    map.entry(10).or_try_insert("TEN".into()).unwrap();
    assert_eq!(map.get(&10), Some(&"ten".into()));

    map.verify_invariants();
}

#[test]
#[serial]
fn rbtree_reverse_duplicate_replaces_value() {
    init();
    let mut map = rb::RbTree::with_comparator(rb::Allocator, Reverse);
    map.try_insert(10, "first".into()).unwrap();
    let old = map.try_insert(10, "second".into()).unwrap();
    assert_eq!(old, Some("first".into()));
    assert_eq!(map.get(&10), Some(&"second".into()));
    assert_eq!(map.len(), 1);
}

// =============================================================================
// BTree with Reverse
// =============================================================================

#[test]
#[serial]
fn btree_reverse_first_last() {
    init();
    let mut map = bt::BTree::with_comparator(bt::Allocator, Reverse);
    map.try_insert(10, "ten".into()).unwrap();
    map.try_insert(30, "thirty".into()).unwrap();
    map.try_insert(20, "twenty".into()).unwrap();

    assert_eq!(map.first_key_value(), Some((&30, &"thirty".into())));
    assert_eq!(map.last_key_value(), Some((&10, &"ten".into())));
    map.verify_invariants();
}

#[test]
#[serial]
fn btree_reverse_pop_first() {
    init();
    let mut map = bt::BTree::with_comparator(bt::Allocator, Reverse);
    for k in [50, 30, 80, 10, 70, 20, 60, 40] {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    map.verify_invariants();

    let mut popped = Vec::new();
    while let Some((k, _)) = map.pop_first() {
        popped.push(k);
        map.verify_invariants();
    }
    assert_eq!(popped, vec![80, 70, 60, 50, 40, 30, 20, 10]);
}

#[test]
#[serial]
fn btree_reverse_iteration_order() {
    init();
    let mut map = bt::BTree::with_comparator(bt::Allocator, Reverse);
    for k in 1..=5 {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.iter().map(|(&k, _)| k).collect();
    assert_eq!(keys, vec![5, 4, 3, 2, 1]);
}

#[test]
#[serial]
fn btree_reverse_get_and_remove() {
    init();
    let mut map = bt::BTree::with_comparator(bt::Allocator, Reverse);
    map.try_insert(10, "ten".into()).unwrap();
    map.try_insert(20, "twenty".into()).unwrap();

    assert_eq!(map.get(&10), Some(&"ten".into()));
    assert_eq!(map.get(&20), Some(&"twenty".into()));
    assert_eq!(map.get(&99), None);

    assert_eq!(map.remove(&10), Some("ten".into()));
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&10), None);
    map.verify_invariants();
}

#[test]
#[serial]
fn btree_reverse_entry_api() {
    init();
    let mut map = bt::BTree::with_comparator(bt::Allocator, Reverse);

    map.entry(10).or_try_insert("ten".into()).unwrap();
    assert_eq!(map.get(&10), Some(&"ten".into()));

    map.entry(10).or_try_insert("TEN".into()).unwrap();
    assert_eq!(map.get(&10), Some(&"ten".into()));

    map.verify_invariants();
}

#[test]
#[serial]
fn btree_reverse_duplicate_replaces_value() {
    init();
    let mut map = bt::BTree::with_comparator(bt::Allocator, Reverse);
    map.try_insert(10, "first".into()).unwrap();
    let old = map.try_insert(10, "second".into()).unwrap();
    assert_eq!(old, Some("first".into()));
    assert_eq!(map.get(&10), Some(&"second".into()));
    assert_eq!(map.len(), 1);
}

// =============================================================================
// Stress: many elements with Reverse to exercise tree balancing
// =============================================================================

#[test]
#[serial]
fn rbtree_reverse_stress() {
    init();
    let mut map = rb::RbTree::with_comparator(rb::Allocator, Reverse);
    for k in 0..100 {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    map.verify_invariants();
    assert_eq!(map.len(), 100);

    // Iteration should be descending.
    let keys: Vec<u64> = map.iter().map(|(&k, _)| k).collect();
    assert_eq!(keys, (0..100).rev().collect::<Vec<_>>());

    // Remove even keys.
    for k in (0..100).step_by(2) {
        map.remove(&k);
    }
    map.verify_invariants();
    assert_eq!(map.len(), 50);

    let keys: Vec<u64> = map.iter().map(|(&k, _)| k).collect();
    assert_eq!(
        keys,
        (0..100).rev().filter(|k| k % 2 != 0).collect::<Vec<_>>()
    );
}

#[test]
#[serial]
fn btree_reverse_stress() {
    init();
    let mut map = bt::BTree::with_comparator(bt::Allocator, Reverse);
    for k in 0..100 {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    map.verify_invariants();
    assert_eq!(map.len(), 100);

    let keys: Vec<u64> = map.iter().map(|(&k, _)| k).collect();
    assert_eq!(keys, (0..100).rev().collect::<Vec<_>>());

    for k in (0..100).step_by(2) {
        map.remove(&k);
    }
    map.verify_invariants();
    assert_eq!(map.len(), 50);

    let keys: Vec<u64> = map.iter().map(|(&k, _)| k).collect();
    assert_eq!(
        keys,
        (0..100).rev().filter(|k| k % 2 != 0).collect::<Vec<_>>()
    );
}
