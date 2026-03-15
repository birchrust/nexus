//! Integration tests for the red-black tree sorted map with internal allocation.

use serial_test::serial;

#[allow(dead_code)]
mod rb {
    nexus_collections::rbtree_allocator!(u64, String, bounded);
}

fn init() {
    let _ = rb::Allocator::builder().capacity(200).build();
}

// =============================================================================
// Basic operations
// =============================================================================

#[test]
#[serial]
fn empty_rbtree() {
    init();
    let map = rb::RbTree::new(rb::Allocator);
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
    assert!(map.first_key_value().is_none());
    assert!(map.last_key_value().is_none());
    map.verify_invariants();
}

#[test]
#[serial]
fn insert_single() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    let old = map.try_insert(10, "ten".into()).unwrap();
    assert!(old.is_none());
    assert_eq!(map.len(), 1);
    assert!(!map.is_empty());
    assert_eq!(map.get(&10), Some(&String::from("ten")));
    map.verify_invariants();
}

#[test]
#[serial]
fn insert_multiple_sorted() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    let keys = [50u64, 30, 80, 10, 70, 20, 60, 40];

    for &k in &keys {
        map.try_insert(k, format!("val-{k}")).unwrap();
        map.verify_invariants();
    }
    assert_eq!(map.len(), 8);

    // Verify sorted iteration.
    let collected: Vec<_> = map.iter().map(|(k, _)| *k).collect();
    assert_eq!(collected, vec![10, 20, 30, 40, 50, 60, 70, 80]);
}

#[test]
#[serial]
fn insert_existing_key_returns_old_value() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    assert!(map.try_insert(10, "first".into()).unwrap().is_none());

    let old = map.try_insert(10, "second".into()).unwrap();
    assert_eq!(old, Some(String::from("first")));

    assert_eq!(map.get(&10), Some(&String::from("second")));
    assert_eq!(map.len(), 1);
    map.verify_invariants();
}

// =============================================================================
// Lookup
// =============================================================================

#[test]
#[serial]
fn get_and_get_mut() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(42, "hello".into()).unwrap();

    assert_eq!(map.get(&42), Some(&String::from("hello")));
    assert!(map.get(&99).is_none());

    *map.get_mut(&42).unwrap() = "world".into();
    assert_eq!(map.get(&42), Some(&String::from("world")));
    map.verify_invariants();
}

#[test]
#[serial]
fn get_key_value() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(42, "hello".into()).unwrap();

    let (k, v) = map.get_key_value(&42).unwrap();
    assert_eq!(*k, 42);
    assert_eq!(v, "hello");
    assert!(map.get_key_value(&99).is_none());
}

#[test]
#[serial]
fn contains_key() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(42, "x".into()).unwrap();

    assert!(map.contains_key(&42));
    assert!(!map.contains_key(&99));
}

#[test]
#[serial]
fn first_and_last() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [50u64, 10, 90, 30, 70] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let (k, v) = map.first_key_value().unwrap();
    assert_eq!(*k, 10);
    assert_eq!(v, "10");

    let (k, v) = map.last_key_value().unwrap();
    assert_eq!(*k, 90);
    assert_eq!(v, "90");
    map.verify_invariants();
}

// =============================================================================
// Remove
// =============================================================================

#[test]
#[serial]
fn remove_by_key() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let removed = map.remove(&30);
    assert_eq!(removed, Some(String::from("30")));
    assert_eq!(map.len(), 4);
    assert!(!map.contains_key(&30));
    map.verify_invariants();

    assert!(map.remove(&99).is_none());

    let keys: Vec<_> = map.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![10, 20, 40, 50]);
}

#[test]
#[serial]
fn remove_entry_by_key() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let (k, v) = map.remove_entry(&20).unwrap();
    assert_eq!(k, 20);
    assert_eq!(v, "20");
    assert_eq!(map.len(), 2);
    map.verify_invariants();
}

#[test]
#[serial]
fn remove_first_and_last() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    map.remove(&10);
    map.verify_invariants();
    assert_eq!(map.first_key_value().unwrap().0, &20);

    map.remove(&30);
    map.verify_invariants();
    assert_eq!(map.last_key_value().unwrap().0, &20);
    assert_eq!(map.len(), 1);
}

// =============================================================================
// Pop
// =============================================================================

#[test]
#[serial]
fn pop_first() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [30u64, 10, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let (k, v) = map.pop_first().unwrap();
    assert_eq!(k, 10);
    assert_eq!(v, "10");
    assert_eq!(map.len(), 2);
    map.verify_invariants();

    let (k, _) = map.pop_first().unwrap();
    assert_eq!(k, 20);
    map.verify_invariants();

    let (k, _) = map.pop_first().unwrap();
    assert_eq!(k, 30);
    map.verify_invariants();

    assert!(map.pop_first().is_none());
    assert!(map.is_empty());
}

#[test]
#[serial]
fn pop_last() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [30u64, 10, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let (k, _) = map.pop_last().unwrap();
    assert_eq!(k, 30);
    assert_eq!(map.len(), 2);
    map.verify_invariants();

    let (k, _) = map.pop_last().unwrap();
    assert_eq!(k, 20);
    map.verify_invariants();

    let (k, _) = map.pop_last().unwrap();
    assert_eq!(k, 10);
    map.verify_invariants();

    assert!(map.pop_last().is_none());
}

// =============================================================================
// Clear and Drop
// =============================================================================

#[test]
#[serial]
fn clear_frees_all() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in 0u64..50 {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    assert_eq!(map.len(), 50);

    map.clear();
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
    assert!(map.first_key_value().is_none());
    assert!(map.last_key_value().is_none());

    // Can reuse allocator slots after clear.
    for k in 0u64..50 {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    assert_eq!(map.len(), 50);
    map.verify_invariants();
}

#[test]
#[serial]
fn drop_frees_all() {
    init();
    {
        let mut map = rb::RbTree::new(rb::Allocator);
        for k in 0u64..50 {
            map.try_insert(k, format!("{k}")).unwrap();
        }
    }
    // Slots should be returned — allocate again to verify.
    {
        let mut map = rb::RbTree::new(rb::Allocator);
        for k in 0u64..50 {
            map.try_insert(k, format!("{k}")).unwrap();
        }
    }
}

// =============================================================================
// Iteration
// =============================================================================

#[test]
#[serial]
fn iter_forward() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [50u64, 30, 10, 40, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let pairs: Vec<_> = map.iter().map(|(k, v)| (*k, v.clone())).collect();
    assert_eq!(
        pairs,
        vec![
            (10, "10".into()),
            (20, "20".into()),
            (30, "30".into()),
            (40, "40".into()),
            (50, "50".into()),
        ]
    );
}

#[test]
#[serial]
fn iter_exact_size() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in 0u64..10 {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let iter = map.iter();
    assert_eq!(iter.len(), 10);
}

#[test]
#[serial]
fn keys_and_values() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [3u64, 1, 2] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<_> = map.keys().copied().collect();
    assert_eq!(keys, vec![1, 2, 3]);

    let values: Vec<_> = map.values().cloned().collect();
    assert_eq!(values, vec!["1", "2", "3"]);
}

// =============================================================================
// Drain
// =============================================================================

#[test]
#[serial]
fn drain_returns_pairs_in_order() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [30u64, 10, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let pairs: Vec<_> = map.drain().collect();
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[0], (10, String::from("10")));
    assert_eq!(pairs[1], (20, String::from("20")));
    assert_eq!(pairs[2], (30, String::from("30")));
    assert!(map.is_empty());
}

#[test]
#[serial]
fn drain_drop_frees_remaining() {
    init();
    {
        let mut map = rb::RbTree::new(rb::Allocator);
        for k in 0u64..50 {
            map.try_insert(k, format!("{k}")).unwrap();
        }

        let mut drain = map.drain();
        let _ = drain.next();
        let _ = drain.next();
        drop(drain);
    }
    // Slots should be available.
    {
        let mut map = rb::RbTree::new(rb::Allocator);
        for k in 0u64..50 {
            map.try_insert(k, format!("{k}")).unwrap();
        }
    }
}

// =============================================================================
// Entry API
// =============================================================================

#[test]
#[serial]
fn entry_vacant_try_insert() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    match map.entry(10) {
        nexus_collections::rbtree::Entry::Vacant(v) => {
            let val = v.try_insert("ten".into()).unwrap();
            assert_eq!(val, "ten");
        }
        nexus_collections::rbtree::Entry::Occupied(_) => panic!("expected vacant"),
    }

    assert_eq!(map.get(&10), Some(&String::from("ten")));
    map.verify_invariants();
}

#[test]
#[serial]
fn entry_occupied_get_and_modify() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(10, "ten".into()).unwrap();

    match map.entry(10) {
        nexus_collections::rbtree::Entry::Occupied(mut o) => {
            assert_eq!(o.get(), "ten");
            *o.get_mut() = "TEN".into();
            assert_eq!(o.get(), "TEN");
        }
        nexus_collections::rbtree::Entry::Vacant(_) => panic!("expected occupied"),
    }

    assert_eq!(map.get(&10), Some(&String::from("TEN")));
}

#[test]
#[serial]
fn entry_occupied_remove() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(10, "ten".into()).unwrap();

    match map.entry(10) {
        nexus_collections::rbtree::Entry::Occupied(o) => {
            let (k, v) = o.remove();
            assert_eq!(k, 10);
            assert_eq!(v, "ten");
        }
        nexus_collections::rbtree::Entry::Vacant(_) => panic!("expected occupied"),
    }

    assert!(map.is_empty());
    map.verify_invariants();
}

#[test]
#[serial]
fn entry_or_try_insert() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    // Vacant — inserts.
    let val = map.entry(10).or_try_insert("ten".into()).unwrap();
    assert_eq!(val, "ten");

    // Occupied — returns existing.
    let val = map.entry(10).or_try_insert("TEN".into()).unwrap();
    assert_eq!(val, "ten");
    map.verify_invariants();
}

#[test]
#[serial]
fn entry_and_modify() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(10, "ten".into()).unwrap();

    map.entry(10)
        .and_modify(|v| *v = "TEN".into())
        .or_try_insert("new".into())
        .unwrap();

    assert_eq!(map.get(&10), Some(&String::from("TEN")));
}

#[test]
#[serial]
fn entry_occupied_insert() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(10, "ten".into()).unwrap();

    match map.entry(10) {
        nexus_collections::rbtree::Entry::Occupied(mut o) => {
            let old = o.insert("TEN".into());
            assert_eq!(old, "ten");
            assert_eq!(o.get(), "TEN");
        }
        nexus_collections::rbtree::Entry::Vacant(_) => panic!("expected occupied"),
    }

    assert_eq!(map.get(&10), Some(&String::from("TEN")));
}

#[test]
#[serial]
fn entry_key() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(10, "ten".into()).unwrap();

    // Occupied entry key.
    assert_eq!(*map.entry(10).key(), 10);

    // Vacant entry key.
    assert_eq!(*map.entry(99).key(), 99);
}

#[test]
#[serial]
fn entry_or_try_insert_with() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    // Vacant — calls closure.
    let val = map.entry(10).or_try_insert_with(|| "ten".into()).unwrap();
    assert_eq!(val, "ten");

    // Occupied — does NOT call closure.
    let val = map
        .entry(10)
        .or_try_insert_with(|| panic!("should not be called"))
        .unwrap();
    assert_eq!(val, "ten");
}

#[test]
#[serial]
fn entry_or_try_insert_with_key() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    let val = map
        .entry(42)
        .or_try_insert_with_key(|k| format!("key={k}"))
        .unwrap();
    assert_eq!(val, "key=42");
}

#[test]
#[serial]
fn entry_or_try_insert_default() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    let val = map.entry(10).or_try_insert_default().unwrap();
    assert_eq!(val, "");

    // Occupied — returns existing.
    *map.get_mut(&10).unwrap() = "ten".into();
    let val = map.entry(10).or_try_insert_default().unwrap();
    assert_eq!(val, "ten");
}

#[test]
#[serial]
fn debug_fmt() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);
    map.try_insert(1, "a".into()).unwrap();
    map.try_insert(2, "b".into()).unwrap();
    let s = format!("{:?}", map);
    assert!(s.contains('1'));
    assert!(s.contains("\"a\""));
    assert!(s.contains('2'));
    assert!(s.contains("\"b\""));
}

// =============================================================================
// Cursor
// =============================================================================

#[test]
#[serial]
fn cursor_front_traversal() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [30u64, 10, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let mut cursor = map.cursor_front();
    assert!(cursor.key().is_none());

    assert!(cursor.advance());
    assert_eq!(cursor.key(), Some(&10));
    assert!(cursor.advance());
    assert_eq!(cursor.key(), Some(&20));
    assert!(cursor.advance());
    assert_eq!(cursor.key(), Some(&30));
    assert!(!cursor.advance());
    assert!(cursor.key().is_none());
}

#[test]
#[serial]
fn cursor_at_key() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let cursor = map.cursor_at(&30);
    assert_eq!(cursor.key(), Some(&30));
    assert_eq!(cursor.value(), Some(&String::from("30")));
}

#[test]
#[serial]
fn cursor_at_nonexistent_key() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let cursor = map.cursor_at(&25);
    assert_eq!(cursor.key(), Some(&30));
}

#[test]
#[serial]
fn cursor_remove() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    {
        let mut cursor = map.cursor_at(&30);
        let (k, v) = cursor.remove().unwrap();
        assert_eq!(k, 30);
        assert_eq!(v, "30");

        // Cursor advanced to next element.
        assert_eq!(cursor.key(), Some(&40));
    }

    assert_eq!(map.len(), 4);
    map.verify_invariants();
    let keys: Vec<_> = map.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![10, 20, 40, 50]);
}

#[test]
#[serial]
fn cursor_value_mut() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let mut cursor = map.cursor_at(&20);
    *cursor.value_mut().unwrap() = "TWENTY".into();
    assert_eq!(cursor.value(), Some(&String::from("TWENTY")));
}

// =============================================================================
// Large-scale
// =============================================================================

#[test]
#[serial]
fn large_sorted_drain() {
    let _ = rb::Allocator::builder().capacity(1100).build();
    let mut map = rb::RbTree::new(rb::Allocator);

    // Insert 1000 elements in shuffled order.
    let mut keys: Vec<u64> = (0..1000).collect();
    for i in (1..keys.len()).rev() {
        let j = (i * 7 + 13) % (i + 1);
        keys.swap(i, j);
    }

    for &k in &keys {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    assert_eq!(map.len(), 1000);
    map.verify_invariants();

    // Drain should produce sorted order.
    let drained: Vec<u64> = map.drain().map(|(k, _)| k).collect();
    let expected: Vec<u64> = (0..1000).collect();
    assert_eq!(drained, expected);
}

// =============================================================================
// Bounded full returns error
// =============================================================================

#[test]
#[serial]
fn bounded_full_returns_error() {
    let _ = rb::Allocator::builder().capacity(3).build();
    let mut map = rb::RbTree::new(rb::Allocator);

    map.try_insert(1, "a".into()).unwrap();
    map.try_insert(2, "b".into()).unwrap();
    map.try_insert(3, "c".into()).unwrap();

    let result = map.try_insert(4, "d".into());
    match result {
        Err(full) => assert_eq!(full.0, (4, String::from("d"))),
        Ok(_) => panic!("expected Full error"),
    }
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
#[serial]
fn insert_remove_reinsert() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    map.try_insert(10, "first".into()).unwrap();
    map.remove(&10);
    assert!(map.is_empty());
    map.verify_invariants();

    map.try_insert(10, "second".into()).unwrap();
    assert_eq!(map.get(&10), Some(&String::from("second")));
    map.verify_invariants();
}

#[test]
#[serial]
fn single_element_operations() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    map.try_insert(42, "x".into()).unwrap();

    assert_eq!(map.first_key_value().unwrap().0, &42);
    assert_eq!(map.last_key_value().unwrap().0, &42);

    let (k, _) = map.pop_first().unwrap();
    assert_eq!(k, 42);
    assert!(map.first_key_value().is_none());
    assert!(map.last_key_value().is_none());
    map.verify_invariants();
}

#[test]
#[serial]
fn iter_empty() {
    init();
    let map = rb::RbTree::new(rb::Allocator);
    assert_eq!(map.iter().count(), 0);
    assert_eq!(map.keys().count(), 0);
    assert_eq!(map.values().count(), 0);
}

// =============================================================================
// iter_mut / values_mut
// =============================================================================

#[test]
#[serial]
fn iter_mut_modify_values() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    for (_, v) in &mut map {
        *v = v.to_uppercase();
    }

    assert_eq!(map.get(&10), Some(&String::from("10")));
    assert_eq!(map.get(&20), Some(&String::from("20")));
    assert_eq!(map.get(&30), Some(&String::from("30")));
}

#[test]
#[serial]
fn values_mut_modify() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [1u64, 2, 3] {
        map.try_insert(k, format!("v{k}")).unwrap();
    }

    for v in map.values_mut() {
        v.push('!');
    }

    assert_eq!(map.get(&1), Some(&String::from("v1!")));
    assert_eq!(map.get(&2), Some(&String::from("v2!")));
    assert_eq!(map.get(&3), Some(&String::from("v3!")));
}

// =============================================================================
// range / range_mut
// =============================================================================

#[test]
#[serial]
fn range_full() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let range_keys: Vec<u64> = map.range(..).map(|(k, _)| *k).collect();
    let iter_keys: Vec<u64> = map.iter().map(|(k, _)| *k).collect();
    assert_eq!(range_keys, iter_keys);
}

#[test]
#[serial]
fn range_inclusive() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(20..=40).map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![20, 30, 40]);
}

#[test]
#[serial]
fn range_exclusive() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(20..40).map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![20, 30]);
}

#[test]
#[serial]
fn range_from() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(30..).map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![30, 40, 50]);
}

#[test]
#[serial]
fn range_to() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(..30).map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![10, 20]);
}

#[test]
#[serial]
fn range_to_inclusive() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(..=30).map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![10, 20, 30]);
}

#[test]
#[serial]
fn range_empty() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(100..=200).map(|(k, _)| *k).collect();
    assert!(keys.is_empty());
}

#[test]
#[serial]
fn range_single() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(20..=20).map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![20]);
}

#[test]
#[serial]
fn range_mut_modify() {
    init();
    let mut map = rb::RbTree::new(rb::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    for (_, v) in map.range_mut(20..=40) {
        v.push('!');
    }

    assert_eq!(map.get(&10), Some(&String::from("10")));
    assert_eq!(map.get(&20), Some(&String::from("20!")));
    assert_eq!(map.get(&30), Some(&String::from("30!")));
    assert_eq!(map.get(&40), Some(&String::from("40!")));
    assert_eq!(map.get(&50), Some(&String::from("50")));
}

// =============================================================================
// Stress tests with invariant verification
// =============================================================================

#[allow(dead_code)]
mod rb_stress {
    nexus_collections::rbtree_allocator!(u64, u64, bounded);
}

/// Stress test: insert many, remove many, verify invariants throughout.
#[test]
#[serial]
fn invariant_stress_insert_remove() {
    let _ = rb_stress::Allocator::builder().capacity(2000).build();

    let mut map = rb_stress::RbTree::new(rb_stress::Allocator);

    // Insert 500 elements in shuffled order.
    let mut keys: Vec<u64> = (0..500).collect();
    for i in (1..keys.len()).rev() {
        let j = (i * 7 + 13) % (i + 1);
        keys.swap(i, j);
    }

    for &k in &keys {
        map.try_insert(k, k).unwrap();
    }
    map.verify_invariants();

    // Remove half (even keys).
    for k in (0..500u64).step_by(2) {
        map.remove(&k);
    }
    map.verify_invariants();

    // Pop from both ends.
    for _ in 0..20 {
        map.pop_first();
        map.pop_last();
    }
    map.verify_invariants();

    // Reinsert.
    for k in 500..700u64 {
        map.try_insert(k, k).unwrap();
    }
    map.verify_invariants();

    // Drain everything.
    let drained: Vec<_> = map.drain().collect();
    assert!(!drained.is_empty());

    // Verify drained is sorted.
    for w in drained.windows(2) {
        assert!(w[0].0 < w[1].0);
    }
}

/// Stress test: ascending and descending insertion (worst case for naive BST).
#[test]
#[serial]
fn invariant_stress_sequential() {
    let _ = rb_stress::Allocator::builder().capacity(2000).build();

    // Ascending.
    {
        let mut map = rb_stress::RbTree::new(rb_stress::Allocator);
        for k in 0..500u64 {
            map.try_insert(k, k).unwrap();
        }
        map.verify_invariants();

        for k in (0..500u64).rev() {
            map.remove(&k);
        }
        assert!(map.is_empty());
    }

    // Descending.
    {
        let mut map = rb_stress::RbTree::new(rb_stress::Allocator);
        for k in (0..500u64).rev() {
            map.try_insert(k, k).unwrap();
        }
        map.verify_invariants();

        for k in 0..500u64 {
            map.remove(&k);
        }
        assert!(map.is_empty());
    }
}

/// Stress test: interleaved insert/remove maintaining steady state.
#[test]
#[serial]
fn invariant_stress_churn() {
    let _ = rb_stress::Allocator::builder().capacity(2000).build();
    let mut map = rb_stress::RbTree::new(rb_stress::Allocator);

    // Build up to 500.
    for k in 0..500u64 {
        map.try_insert(k, k).unwrap();
    }

    // Churn: remove old, insert new.
    for k in 500..1500u64 {
        map.remove(&(k - 500));
        map.try_insert(k, k).unwrap();
    }
    map.verify_invariants();
    assert_eq!(map.len(), 500);

    // Verify sorted.
    let keys: Vec<u64> = map.iter().map(|(k, _)| *k).collect();
    let expected: Vec<u64> = (1000..1500).collect();
    assert_eq!(keys, expected);
}

/// Stress test: verify invariants after every single mutation.
#[test]
#[serial]
fn invariant_every_mutation() {
    let _ = rb_stress::Allocator::builder().capacity(2000).build();
    let mut map = rb_stress::RbTree::new(rb_stress::Allocator);

    // Insert 100, verifying after each.
    for k in 0..100u64 {
        map.try_insert(k, k).unwrap();
        map.verify_invariants();
    }

    // Remove in random-ish order, verifying after each.
    let mut keys: Vec<u64> = (0..100).collect();
    for i in (1..keys.len()).rev() {
        let j = (i * 11 + 7) % (i + 1);
        keys.swap(i, j);
    }

    for &k in &keys {
        map.remove(&k);
        map.verify_invariants();
    }
    assert!(map.is_empty());
}
