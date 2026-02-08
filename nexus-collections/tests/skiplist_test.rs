//! Integration tests for the skip list sorted map with internal allocation.

use serial_test::serial;

#[allow(dead_code)]
mod sl {
    nexus_collections::skip_allocator!(u64, String, bounded);
}

fn init() {
    let _ = sl::Allocator::builder().capacity(200).build();
}

// =============================================================================
// Basic operations
// =============================================================================

#[test]
#[serial]
fn empty_skip_list() {
    init();
    let map = sl::SkipList::new(sl::Allocator);
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
    assert!(map.first_key_value().is_none());
    assert!(map.last_key_value().is_none());
}

#[test]
#[serial]
fn insert_single() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);
    let old = map.try_insert(10, "ten".into()).unwrap();
    assert!(old.is_none());
    assert_eq!(map.len(), 1);
    assert!(!map.is_empty());
    assert_eq!(map.get(&10), Some(&String::from("ten")));
}

#[test]
#[serial]
fn insert_multiple_sorted() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);
    let keys = [50u64, 30, 80, 10, 70, 20, 60, 40];

    for &k in &keys {
        map.try_insert(k, format!("val-{k}")).unwrap();
    }
    assert_eq!(map.len(), 8);

    // Verify sorted iteration
    let collected: Vec<_> = map.iter().map(|(k, _)| *k).collect();
    assert_eq!(collected, vec![10, 20, 30, 40, 50, 60, 70, 80]);
}

#[test]
#[serial]
fn insert_existing_key_returns_old_value() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    assert!(map.try_insert(10, "first".into()).unwrap().is_none());

    let old = map.try_insert(10, "second".into()).unwrap();
    assert_eq!(old, Some(String::from("first")));

    // Map has the updated value
    assert_eq!(map.get(&10), Some(&String::from("second")));
    assert_eq!(map.len(), 1);
}

// =============================================================================
// Lookup
// =============================================================================

#[test]
#[serial]
fn get_and_get_mut() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);
    map.try_insert(42, "hello".into()).unwrap();

    assert_eq!(map.get(&42), Some(&String::from("hello")));
    assert!(map.get(&99).is_none());

    *map.get_mut(&42).unwrap() = "world".into();
    assert_eq!(map.get(&42), Some(&String::from("world")));
}

#[test]
#[serial]
fn get_key_value() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);
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
    let mut map = sl::SkipList::new(sl::Allocator);
    map.try_insert(42, "x".into()).unwrap();

    assert!(map.contains_key(&42));
    assert!(!map.contains_key(&99));
}

#[test]
#[serial]
fn first_and_last() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [50u64, 10, 90, 30, 70] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let (k, v) = map.first_key_value().unwrap();
    assert_eq!(*k, 10);
    assert_eq!(v, "10");

    let (k, v) = map.last_key_value().unwrap();
    assert_eq!(*k, 90);
    assert_eq!(v, "90");
}

// =============================================================================
// Remove
// =============================================================================

#[test]
#[serial]
fn remove_by_key() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    // Remove middle element
    let removed = map.remove(&30);
    assert_eq!(removed, Some(String::from("30")));
    assert_eq!(map.len(), 4);
    assert!(!map.contains_key(&30));

    // Remove non-existent
    assert!(map.remove(&99).is_none());

    // Remaining elements still sorted
    let keys: Vec<_> = map.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![10, 20, 40, 50]);
}

#[test]
#[serial]
fn remove_entry_by_key() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let (k, v) = map.remove_entry(&20).unwrap();
    assert_eq!(k, 20);
    assert_eq!(v, "20");
    assert_eq!(map.len(), 2);
}

#[test]
#[serial]
fn remove_first_and_last() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    map.remove(&10);
    assert_eq!(map.first_key_value().unwrap().0, &20);

    map.remove(&30);
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
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [30u64, 10, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let (k, v) = map.pop_first().unwrap();
    assert_eq!(k, 10);
    assert_eq!(v, "10");
    assert_eq!(map.len(), 2);

    let (k, _) = map.pop_first().unwrap();
    assert_eq!(k, 20);

    let (k, _) = map.pop_first().unwrap();
    assert_eq!(k, 30);

    assert!(map.pop_first().is_none());
    assert!(map.is_empty());
}

#[test]
#[serial]
fn pop_last() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [30u64, 10, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let (k, _) = map.pop_last().unwrap();
    assert_eq!(k, 30);
    assert_eq!(map.len(), 2);

    let (k, _) = map.pop_last().unwrap();
    assert_eq!(k, 20);

    let (k, _) = map.pop_last().unwrap();
    assert_eq!(k, 10);

    assert!(map.pop_last().is_none());
}

// =============================================================================
// Clear and Drop
// =============================================================================

#[test]
#[serial]
fn clear_frees_all() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in 0u64..50 {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    assert_eq!(map.len(), 50);

    map.clear();
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
    assert!(map.first_key_value().is_none());
    assert!(map.last_key_value().is_none());

    // Can reuse allocator slots after clear
    for k in 0u64..50 {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    assert_eq!(map.len(), 50);
}

#[test]
#[serial]
fn drop_frees_all() {
    init();
    {
        let mut map = sl::SkipList::new(sl::Allocator);
        for k in 0u64..50 {
            map.try_insert(k, format!("{k}")).unwrap();
        }
    }
    // Slots should be returned — allocate again to verify
    {
        let mut map = sl::SkipList::new(sl::Allocator);
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
    let mut map = sl::SkipList::new(sl::Allocator);

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
fn iter_reverse() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [50u64, 30, 10, 40, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let pairs: Vec<_> = map.iter().rev().map(|(k, _)| *k).collect();
    assert_eq!(pairs, vec![50, 40, 30, 20, 10]);
}

#[test]
#[serial]
fn iter_exact_size() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

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
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [3u64, 1, 2] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<_> = map.keys().copied().collect();
    assert_eq!(keys, vec![1, 2, 3]);

    let values: Vec<_> = map.values().cloned().collect();
    assert_eq!(values, vec!["1", "2", "3"]);
}

#[test]
#[serial]
fn iter_double_ended_meets_in_middle() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in 1u64..=5 {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let mut iter = map.iter();
    assert_eq!(iter.next().unwrap().0, &1);
    assert_eq!(iter.next_back().unwrap().0, &5);
    assert_eq!(iter.next().unwrap().0, &2);
    assert_eq!(iter.next_back().unwrap().0, &4);
    assert_eq!(iter.next().unwrap().0, &3);
    assert!(iter.next().is_none());
}

// =============================================================================
// Drain
// =============================================================================

#[test]
#[serial]
fn drain_returns_pairs_in_order() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

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
        let mut map = sl::SkipList::new(sl::Allocator);
        for k in 0u64..50 {
            map.try_insert(k, format!("{k}")).unwrap();
        }

        let mut drain = map.drain();
        // Only consume a few
        let _ = drain.next();
        let _ = drain.next();
        // Drop the drain — remaining should be freed
        drop(drain);
    }
    // Slots should be available
    {
        let mut map = sl::SkipList::new(sl::Allocator);
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
    let mut map = sl::SkipList::new(sl::Allocator);

    match map.entry(10) {
        nexus_collections::skiplist::Entry::Vacant(v) => {
            let val = v.try_insert("ten".into()).unwrap();
            assert_eq!(val, "ten");
        }
        nexus_collections::skiplist::Entry::Occupied(_) => panic!("expected vacant"),
    }

    assert_eq!(map.get(&10), Some(&String::from("ten")));
}

#[test]
#[serial]
fn entry_occupied_get_and_modify() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);
    map.try_insert(10, "ten".into()).unwrap();

    match map.entry(10) {
        nexus_collections::skiplist::Entry::Occupied(mut o) => {
            assert_eq!(o.get(), "ten");
            *o.get_mut() = "TEN".into();
            assert_eq!(o.get(), "TEN");
        }
        nexus_collections::skiplist::Entry::Vacant(_) => panic!("expected occupied"),
    }

    assert_eq!(map.get(&10), Some(&String::from("TEN")));
}

#[test]
#[serial]
fn entry_occupied_remove() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);
    map.try_insert(10, "ten".into()).unwrap();

    match map.entry(10) {
        nexus_collections::skiplist::Entry::Occupied(o) => {
            let (k, v) = o.remove();
            assert_eq!(k, 10);
            assert_eq!(v, "ten");
        }
        nexus_collections::skiplist::Entry::Vacant(_) => panic!("expected occupied"),
    }

    assert!(map.is_empty());
}

#[test]
#[serial]
fn entry_or_try_insert() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    // Vacant — inserts
    let val = map.entry(10).or_try_insert("ten".into()).unwrap();
    assert_eq!(val, "ten");

    // Occupied — returns existing
    let val = map.entry(10).or_try_insert("TEN".into()).unwrap();
    assert_eq!(val, "ten");
}

#[test]
#[serial]
fn entry_and_modify() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);
    map.try_insert(10, "ten".into()).unwrap();

    map.entry(10)
        .and_modify(|v| *v = "TEN".into())
        .or_try_insert("new".into())
        .unwrap();

    assert_eq!(map.get(&10), Some(&String::from("TEN")));
}

// =============================================================================
// Cursor
// =============================================================================

#[test]
#[serial]
fn cursor_front_traversal() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [30u64, 10, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    // cursor_front() starts before the first element
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
fn cursor_backward_traversal() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [30u64, 10, 20] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let mut cursor = map.cursor_front();
    // Walk to past the end
    while cursor.advance() {}
    assert!(cursor.key().is_none());

    // advance_back() from past-end goes to tail
    assert!(cursor.advance_back());
    assert_eq!(cursor.key(), Some(&30));
    assert!(cursor.advance_back());
    assert_eq!(cursor.key(), Some(&20));
    assert!(cursor.advance_back());
    assert_eq!(cursor.key(), Some(&10));
    assert!(!cursor.advance_back());
    assert!(cursor.key().is_none());
}

#[test]
#[serial]
fn cursor_at_key() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

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
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    // cursor_at positions at first element > key when key not found
    let cursor = map.cursor_at(&25);
    assert_eq!(cursor.key(), Some(&30));
}

#[test]
#[serial]
fn cursor_remove() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    {
        let mut cursor = map.cursor_at(&30);
        let (k, v) = cursor.remove().unwrap();
        assert_eq!(k, 30);
        assert_eq!(v, "30");

        // Cursor advanced to next element
        assert_eq!(cursor.key(), Some(&40));
    }

    assert_eq!(map.len(), 4);
    let keys: Vec<_> = map.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![10, 20, 40, 50]);
}

#[test]
#[serial]
fn cursor_value_mut() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

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
    let _ = sl::Allocator::builder().capacity(1100).build();
    let mut map = sl::SkipList::new(sl::Allocator);

    // Insert 1000 elements in shuffled order
    let mut keys: Vec<u64> = (0..1000).collect();
    // Simple deterministic shuffle
    for i in (1..keys.len()).rev() {
        let j = (i * 7 + 13) % (i + 1);
        keys.swap(i, j);
    }

    for &k in &keys {
        map.try_insert(k, format!("{k}")).unwrap();
    }
    assert_eq!(map.len(), 1000);

    // Drain should produce sorted order
    let drained: Vec<u64> = map.drain().map(|(k, _)| k).collect();
    let expected: Vec<u64> = (0..1000).collect();
    assert_eq!(drained, expected);
}

// =============================================================================
// Level ratio tuning
// =============================================================================

#[test]
#[serial]
fn level_ratio_tuning() {
    init();

    // p=0.5 (default, level_ratio=2)
    let mut map1 = sl::SkipList::new(sl::Allocator);
    for k in 0u64..100 {
        map1.try_insert(k, String::new()).unwrap();
    }

    // Verify it works (sorted output)
    let keys: Vec<_> = map1.iter().map(|(k, _)| *k).collect();
    let expected: Vec<u64> = (0..100).collect();
    assert_eq!(keys, expected);
    map1.clear();

    // p=0.25 (RATIO=4) — same allocator (MAX_LEVEL=8), different promotion ratio
    let mut map2 =
        nexus_collections::skiplist::SkipList::<u64, String, sl::Allocator, 8, 4>::new(
            sl::Allocator,
        );
    for k in 0u64..100 {
        map2.try_insert(k, String::new()).unwrap();
    }

    let keys: Vec<_> = map2.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, expected);
}

// =============================================================================
// Bounded full returns error
// =============================================================================

#[test]
#[serial]
fn bounded_full_returns_error() {
    let _ = sl::Allocator::builder().capacity(3).build();
    let mut map = sl::SkipList::new(sl::Allocator);

    map.try_insert(1, "a".into()).unwrap();
    map.try_insert(2, "b".into()).unwrap();
    map.try_insert(3, "c".into()).unwrap();

    // Allocator is full
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
    let mut map = sl::SkipList::new(sl::Allocator);

    map.try_insert(10, "first".into()).unwrap();

    map.remove(&10);
    assert!(map.is_empty());

    map.try_insert(10, "second".into()).unwrap();
    assert_eq!(map.get(&10), Some(&String::from("second")));
}

#[test]
#[serial]
fn single_element_operations() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    map.try_insert(42, "x".into()).unwrap();

    assert_eq!(map.first_key_value().unwrap().0, &42);
    assert_eq!(map.last_key_value().unwrap().0, &42);

    let (k, _) = map.pop_first().unwrap();
    assert_eq!(k, 42);
    assert!(map.first_key_value().is_none());
    assert!(map.last_key_value().is_none());
}

#[test]
#[serial]
fn iter_empty() {
    init();
    let map = sl::SkipList::new(sl::Allocator);
    assert_eq!(map.iter().count(), 0);
    assert_eq!(map.iter().rev().count(), 0);
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
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    for (_, v) in map.iter_mut() {
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
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [1u64, 2, 3] {
        map.try_insert(k, format!("v{k}")).unwrap();
    }

    for v in map.values_mut() {
        v.push_str("!");
    }

    assert_eq!(map.get(&1), Some(&String::from("v1!")));
    assert_eq!(map.get(&2), Some(&String::from("v2!")));
    assert_eq!(map.get(&3), Some(&String::from("v3!")));
}

#[test]
#[serial]
fn iter_mut_double_ended() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in 1u64..=5 {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let mut iter = map.iter_mut();
    assert_eq!(iter.next().unwrap().0, &1);
    assert_eq!(iter.next_back().unwrap().0, &5);
    assert_eq!(iter.next().unwrap().0, &2);
    assert_eq!(iter.next_back().unwrap().0, &4);
    assert_eq!(iter.next().unwrap().0, &3);
    assert!(iter.next().is_none());
}

// =============================================================================
// range / range_mut
// =============================================================================

#[test]
#[serial]
fn range_full() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

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
    let mut map = sl::SkipList::new(sl::Allocator);

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
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    // 20..40 includes 20, 30 but not 40
    let keys: Vec<u64> = map.range(20..40).map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![20, 30]);
}

#[test]
#[serial]
fn range_from() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

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
    let mut map = sl::SkipList::new(sl::Allocator);

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
    let mut map = sl::SkipList::new(sl::Allocator);

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
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    // No elements in [100, 200]
    let keys: Vec<u64> = map.range(100..=200).map(|(k, _)| *k).collect();
    assert!(keys.is_empty());
}

#[test]
#[serial]
fn range_single() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(20..=20).map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![20]);
}

#[test]
#[serial]
fn range_reverse() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range(20..=40).rev().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![40, 30, 20]);
}

#[test]
#[serial]
fn range_meets_in_middle() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let mut range = map.range(10..=50);
    assert_eq!(range.next().unwrap().0, &10);
    assert_eq!(range.next_back().unwrap().0, &50);
    assert_eq!(range.next().unwrap().0, &20);
    assert_eq!(range.next_back().unwrap().0, &40);
    assert_eq!(range.next().unwrap().0, &30);
    assert!(range.next().is_none());
    assert!(range.next_back().is_none());
}

#[test]
#[serial]
fn range_mut_modify() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    for (_, v) in map.range_mut(20..=40) {
        v.push_str("!");
    }

    assert_eq!(map.get(&10), Some(&String::from("10")));
    assert_eq!(map.get(&20), Some(&String::from("20!")));
    assert_eq!(map.get(&30), Some(&String::from("30!")));
    assert_eq!(map.get(&40), Some(&String::from("40!")));
    assert_eq!(map.get(&50), Some(&String::from("50")));
}

#[test]
#[serial]
fn range_mut_reverse() {
    init();
    let mut map = sl::SkipList::new(sl::Allocator);

    for k in [10u64, 20, 30, 40, 50] {
        map.try_insert(k, format!("{k}")).unwrap();
    }

    let keys: Vec<u64> = map.range_mut(20..=40).rev().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![40, 30, 20]);
}

// =============================================================================
// Level bound stress test
// =============================================================================

#[allow(dead_code)]
mod sl_stress {
    nexus_collections::skip_allocator!(u64, u64, bounded);
}

#[allow(dead_code)]
mod sl_stress_r4 {
    nexus_collections::skip_allocator!(u64, u64, bounded, 8, 4);
}

/// Stress test that exercises level generation with multiple seeds.
///
/// Runs in debug mode where `debug_assert!` in find/search/link_node/
/// unlink_at_levels/pop_first fires if any level bound is violated.
/// This validates the `assert_unchecked` hints are sound.
#[test]
#[serial]
fn level_bounds_stress() {
    let _ = sl_stress::Allocator::builder().capacity(2000).build();

    // Multiple seeds to maximize RNG coverage of level generation
    for seed in [1u64, 42, 0xDEAD_BEEF, 0xCAFE_BABE, u64::MAX] {
        let mut map = sl_stress::SkipList::with_seed(seed, sl_stress::Allocator);

        // Insert enough to force high levels (2^8 = 256 capacity)
        for k in 0..200u64 {
            map.try_insert(k, k).unwrap();
        }

        // Exercise every search path
        for k in 0..200u64 {
            assert_eq!(map.get(&k), Some(&k));
            assert!(map.contains_key(&k));
        }

        // Remove half (exercises unlink_at_levels + update_tail_and_level)
        for k in (0..200u64).step_by(2) {
            assert!(map.remove(&k).is_some());
        }

        // Pop from both ends
        for _ in 0..10 {
            map.pop_first();
            map.pop_last();
        }

        // Reinsert to force relinking at potentially different levels
        for k in 200..300u64 {
            map.try_insert(k, k).unwrap();
        }

        // Drain everything — exercises pop_first repeatedly down to empty
        let drained: Vec<_> = map.drain().collect();
        assert!(!drained.is_empty());
    }

    // Also test with RATIO=4
    {
        let _ = sl_stress_r4::Allocator::builder().capacity(2000).build();
        let mut map = sl_stress_r4::SkipList::with_seed(0xBEEF, sl_stress_r4::Allocator);

        for k in 0..500u64 {
            map.try_insert(k, k).unwrap();
        }

        for k in (0..500u64).rev() {
            map.remove(&k);
        }

        assert!(map.is_empty());
    }
}
