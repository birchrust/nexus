//! Property-based tests for slab invariants.

use proptest::prelude::*;

use nexus_slab::{BoundedSlab, Key, Slab};

// =============================================================================
// BoundedSlab properties
// =============================================================================

proptest! {
    /// len() always equals the number of occupied slots
    #[test]
    fn bounded_len_invariant(ops in prop::collection::vec(0..100u64, 0..200)) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(100);
        let mut keys: Vec<Key> = Vec::new();
        let mut expected_len = 0;

        for op in ops {
            if op % 3 == 0 && !keys.is_empty() {
                // Remove
                let idx = (op as usize) % keys.len();
                let key = keys.remove(idx);
                slab.remove_by_key(key);
                expected_len -= 1;
            } else if expected_len < 100 {
                // Insert
                if let Ok(entry) = slab.try_insert(op) {
                    keys.push(entry.leak());
                    expected_len += 1;
                }
            }

            prop_assert_eq!(slab.len(), expected_len);
        }
    }

    /// Capacity is never exceeded
    #[test]
    fn bounded_capacity_never_exceeded(values in prop::collection::vec(0..1000u64, 0..500)) {
        let capacity = 100;
        let slab: BoundedSlab<u64> = BoundedSlab::leak(capacity);

        for value in values {
            if slab.len() < capacity {
                prop_assert!(slab.try_insert(value).is_ok());
            } else {
                prop_assert!(slab.try_insert(value).is_err());
            }
        }

        prop_assert!(slab.len() <= capacity);
    }

    /// Insert returns the same value on remove
    #[test]
    fn bounded_insert_remove_roundtrip(values in prop::collection::vec(0..10000u64, 1..100)) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(values.len());

        for value in values {
            let entry = slab.try_insert(value).unwrap();
            prop_assert_eq!(*entry.get(), value);
            prop_assert_eq!(entry.remove(), value);
        }
    }

    /// Keys remain valid until removed
    #[test]
    fn bounded_key_validity(values in prop::collection::vec(0..1000u64, 1..50)) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(values.len());
        let mut key_value_pairs = Vec::new();

        // Insert all
        for value in &values {
            let entry = slab.try_insert(*value).unwrap();
            key_value_pairs.push((entry.leak(), *value));
        }

        // All keys should be valid
        for (key, expected) in &key_value_pairs {
            prop_assert!(slab.contains_key(*key));
            prop_assert_eq!(*slab.get(*key).unwrap(), *expected);
        }

        // Remove half
        for (key, expected) in key_value_pairs.iter().take(values.len() / 2) {
            prop_assert_eq!(slab.remove_by_key(*key).unwrap(), *expected);
            prop_assert!(!slab.contains_key(*key));
        }

        // Remaining keys still valid
        for (key, expected) in key_value_pairs.iter().skip(values.len() / 2) {
            prop_assert!(slab.contains_key(*key));
            prop_assert_eq!(*slab.get(*key).unwrap(), *expected);
        }
    }

    /// LIFO: most recently freed slot is reused first
    #[test]
    fn bounded_lifo_freelist(n in 2..50usize) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(n);

        // Fill the slab
        let mut keys: Vec<Key> = Vec::new();
        for i in 0..n {
            let entry = slab.try_insert(i as u64).unwrap();
            keys.push(entry.leak());
        }

        // Remove two slots: first_removed, then second_removed
        // LIFO = Last In First Out, so second_removed (last to enter freelist)
        // should be allocated first
        let first_removed = keys.pop().unwrap();
        let second_removed = keys.pop().unwrap();
        slab.remove_by_key(first_removed);   // freelist: first_removed
        slab.remove_by_key(second_removed);  // freelist: second_removed -> first_removed

        // Next insert should get second_removed (most recently added to freelist)
        let new_entry1 = slab.try_insert(999u64).unwrap();
        prop_assert_eq!(new_entry1.key(), second_removed);

        // Next insert should get first_removed
        let new_entry2 = slab.try_insert(998u64).unwrap();
        prop_assert_eq!(new_entry2.key(), first_removed);
    }

    /// Replace preserves slot, changes value
    #[test]
    fn bounded_replace_preserves_slot(values in prop::collection::vec(1..1000u64, 2..50)) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(values.len());

        let entry = slab.try_insert(0u64).unwrap();
        let original_key = entry.key();
        let mut expected_old = 0u64;

        for value in values {
            let old = entry.replace(value);
            prop_assert_eq!(entry.key(), original_key); // Key preserved
            prop_assert_eq!(old, expected_old);         // Got previous value
            prop_assert_eq!(*entry.get(), value);       // New value set
            expected_old = value;
        }
    }

    /// Clear empties slab completely
    #[test]
    fn bounded_clear_empties(values in prop::collection::vec(0..1000u64, 1..100)) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(values.len());

        for value in &values {
            slab.try_insert(*value).unwrap().leak();
        }

        prop_assert_eq!(slab.len(), values.len());
        slab.clear();
        prop_assert_eq!(slab.len(), 0);
        prop_assert!(slab.is_empty());
    }
}

// =============================================================================
// Slab (unbounded) properties
// =============================================================================

proptest! {
    /// len() always equals the number of occupied slots
    #[test]
    fn unbounded_len_invariant(ops in prop::collection::vec(0..100u64, 0..200)) {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(16).build();
        let mut keys: Vec<Key> = Vec::new();
        let mut expected_len = 0;

        for op in ops {
            if op % 3 == 0 && !keys.is_empty() {
                // Remove
                let idx = (op as usize) % keys.len();
                let key = keys.remove(idx);
                slab.remove_by_key(key);
                expected_len -= 1;
            } else {
                // Insert (always succeeds for unbounded)
                keys.push(slab.insert(op).leak());
                expected_len += 1;
            }

            prop_assert_eq!(slab.len(), expected_len);
        }
    }

    /// Insert returns the same value on remove
    #[test]
    fn unbounded_insert_remove_roundtrip(values in prop::collection::vec(0..10000u64, 1..100)) {
        let slab = Slab::with_capacity(values.len());

        for value in values {
            let entry = slab.insert(value);
            prop_assert_eq!(*entry.get(), value);
            prop_assert_eq!(entry.remove(), value);
        }
    }

    /// Keys remain valid until removed
    #[test]
    fn unbounded_key_validity(values in prop::collection::vec(0..1000u64, 1..50)) {
        let slab = Slab::with_capacity(values.len());
        let mut key_value_pairs = Vec::new();

        // Insert all
        for value in &values {
            let entry = slab.insert(*value);
            key_value_pairs.push((entry.leak(), *value));
        }

        // All keys should be valid
        for (key, expected) in &key_value_pairs {
            prop_assert!(slab.contains_key(*key));
            prop_assert_eq!(*slab.get(*key).unwrap(), *expected);
        }

        // Remove half
        for (key, expected) in key_value_pairs.iter().take(values.len() / 2) {
            prop_assert_eq!(slab.remove_by_key(*key).unwrap(), *expected);
            prop_assert!(!slab.contains_key(*key));
        }

        // Remaining keys still valid
        for (key, expected) in key_value_pairs.iter().skip(values.len() / 2) {
            prop_assert!(slab.contains_key(*key));
            prop_assert_eq!(*slab.get(*key).unwrap(), *expected);
        }
    }

    /// Growth: slab can exceed initial capacity
    #[test]
    fn unbounded_grows_beyond_capacity(n in 10..200usize) {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(8).build();

        // Insert more than a single chunk
        for i in 0..n {
            slab.insert(i as u64).leak();
        }

        prop_assert_eq!(slab.len(), n);
        prop_assert!(slab.capacity() >= n);
    }

    /// Replace preserves slot, changes value
    #[test]
    fn unbounded_replace_preserves_slot(values in prop::collection::vec(1..1000u64, 2..50)) {
        let slab = Slab::with_capacity(values.len());

        let entry = slab.insert(0u64);
        let original_key = entry.key();
        let mut expected_old = 0u64;

        for value in values {
            let old = entry.replace(value);
            prop_assert_eq!(entry.key(), original_key); // Key preserved
            prop_assert_eq!(old, expected_old);         // Got previous value
            prop_assert_eq!(*entry.get(), value);       // New value set
            expected_old = value;
        }
    }

    /// Clear empties slab completely
    #[test]
    fn unbounded_clear_empties(values in prop::collection::vec(0..1000u64, 1..100)) {
        let slab = Slab::with_capacity(values.len());

        for value in &values {
            slab.insert(*value).leak();
        }

        prop_assert_eq!(slab.len(), values.len());
        slab.clear();
        prop_assert_eq!(slab.len(), 0);
        prop_assert!(slab.is_empty());
    }

    /// Multi-chunk: values distributed across chunks remain accessible
    #[test]
    fn unbounded_multi_chunk_access(values in prop::collection::vec(0..10000u64, 50..150)) {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(16).build();
        let mut key_value_pairs = Vec::new();

        // Insert all (will span multiple chunks)
        for value in &values {
            let entry = slab.insert(*value);
            key_value_pairs.push((entry.leak(), *value));
        }

        // All should be accessible
        for (key, expected) in &key_value_pairs {
            prop_assert_eq!(*slab.get(*key).unwrap(), *expected);
        }
    }
}

// =============================================================================
// Entry invariants
// =============================================================================

proptest! {
    /// Entry::is_valid reflects actual state after key-based removal
    #[test]
    fn entry_is_valid_reflects_state(values in prop::collection::vec(0..100u64, 1..20)) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(values.len());
        let mut keys = Vec::new();

        for value in &values {
            let entry = slab.try_insert(*value).unwrap();
            keys.push(entry.leak());
        }

        // Remove every other via key
        for (i, key) in keys.iter().enumerate() {
            if i % 2 == 0 {
                slab.remove_by_key(*key);
            }
        }

        // Check validity via slab.entry()
        for (i, key) in keys.iter().enumerate() {
            if i % 2 == 0 {
                prop_assert!(slab.entry(*key).is_none());
            } else {
                prop_assert!(slab.entry(*key).is_some());
            }
        }
    }

    /// VacantEntry key matches final Entry key
    #[test]
    fn vacant_entry_key_matches(n in 1..50usize) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(n);

        for _ in 0..n {
            let vacant = slab.try_vacant_entry().unwrap();
            let expected_key = vacant.key();
            let entry = vacant.insert(42u64);
            prop_assert_eq!(entry.key(), expected_key);
            entry.remove();
        }
    }

    /// take() returns value and allows re-insert at same slot
    #[test]
    fn take_preserves_slot(values in prop::collection::vec(0..1000u64, 1..20)) {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(values.len());
        let entry = slab.try_insert(values[0]).unwrap();
        let original_key = entry.key();

        let (value, vacant) = entry.take();
        prop_assert_eq!(value, values[0]);
        prop_assert_eq!(vacant.key(), original_key);

        let new_entry = vacant.insert(values.get(1).copied().unwrap_or(999));
        prop_assert_eq!(new_entry.key(), original_key);
    }
}
