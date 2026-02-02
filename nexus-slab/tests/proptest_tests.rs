//! Property-based tests using proptest.
//!
//! These tests verify invariants hold under randomized inputs:
//! - len() always matches actual occupied slots
//! - contains_key() accurately reflects slot state
//! - Freelist maintains integrity under arbitrary insert/remove sequences
//! - Values are never corrupted
//!
//! Note: Due to TLS persistence, each proptest uses a uniquely named allocator
//! to avoid reinitialization issues across test iterations.

use nexus_slab::{Key, create_allocator};
use proptest::prelude::*;
use std::collections::HashMap;

// =============================================================================
// Key Properties (stateless, no allocator needed)
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn key_roundtrip(raw in 0u32..nexus_slab::SLOT_NONE) {
        let key = Key::from_raw(raw);
        prop_assert_eq!(key.into_raw(), raw);
        prop_assert_eq!(key.index(), raw);
    }

    #[test]
    fn key_valid_is_some(raw in 0u32..nexus_slab::SLOT_NONE) {
        let key = Key::from_raw(raw);
        prop_assert!(key.is_some());
        prop_assert!(!key.is_none());
    }
}

#[test]
fn key_none_is_special() {
    let key = Key::NONE;
    assert!(key.is_none());
    assert!(!key.is_some());
    assert_eq!(key.index(), nexus_slab::SLOT_NONE);
}

// =============================================================================
// Bounded Slab Properties
// Each test creates a uniquely-named allocator to avoid TLS conflicts.
// =============================================================================

/// Test that len() always matches actual occupied slots
#[test]
fn bounded_len_invariant_random() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(50).build();

    let mut rng = proptest::test_runner::TestRng::deterministic_rng(
        proptest::test_runner::RngAlgorithm::ChaCha
    );

    let mut slots = Vec::new();
    let mut leaked_count = 0usize;

    for _ in 0..500 {
        let action: u8 = rng.random_range(0..10);

        match action {
            0..=5 => {
                // Insert
                if let Some(slot) = test_alloc::try_insert(rng.random()) {
                    slots.push(slot);
                }
            }
            6..=7 => {
                // Remove last
                let _ = slots.pop();
            }
            8 => {
                // Remove random
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let _ = slots.swap_remove(idx);
                }
            }
            9 => {
                // Leak random
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let slot = slots.swap_remove(idx);
                    let _ = slot.leak();
                    leaked_count += 1;
                }
            }
            _ => unreachable!(),
        }

        // Invariant: len() == slots.len() + leaked_count
        assert_eq!(test_alloc::len(), slots.len() + leaked_count);
    }
}

/// Test that contains_key() accurately reflects slot state
#[test]
fn bounded_contains_key_invariant_random() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(50).build();

    let mut rng = proptest::test_runner::TestRng::deterministic_rng(
        proptest::test_runner::RngAlgorithm::ChaCha
    );

    let mut slots = Vec::new();
    let mut leaked_keys = Vec::new();

    for _ in 0..500 {
        let action: u8 = rng.random_range(0..10);

        match action {
            0..=5 => {
                if let Some(slot) = test_alloc::try_insert(rng.random()) {
                    slots.push(slot);
                }
            }
            6..=7 => {
                let _ = slots.pop();
            }
            8 => {
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let _ = slots.swap_remove(idx);
                }
            }
            9 => {
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let slot = slots.swap_remove(idx);
                    leaked_keys.push(slot.leak());
                }
            }
            _ => unreachable!(),
        }

        // Invariant: contains_key returns true for all held slots
        for slot in &slots {
            assert!(test_alloc::contains_key(slot.key()));
        }

        // Invariant: contains_key returns true for all leaked keys
        for &key in &leaked_keys {
            assert!(test_alloc::contains_key(key));
        }
    }
}

/// Test that values are never corrupted
#[test]
fn bounded_value_integrity_random() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(50).build();

    let mut rng = proptest::test_runner::TestRng::deterministic_rng(
        proptest::test_runner::RngAlgorithm::ChaCha
    );

    // Track expected values
    let mut slots: Vec<(_, u64)> = Vec::new();

    for _ in 0..500 {
        let action: u8 = rng.random_range(0..10);

        match action {
            0..=5 => {
                let value: u64 = rng.random();
                if let Some(slot) = test_alloc::try_insert(value) {
                    slots.push((slot, value));
                }
            }
            6..=7 => {
                let _ = slots.pop();
            }
            8 => {
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let _ = slots.swap_remove(idx);
                }
            }
            9 => {
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let new_value: u64 = rng.random();
                    slots[idx].0.replace(new_value);
                    slots[idx].1 = new_value;
                }
            }
            _ => unreachable!(),
        }

        // Invariant: all values match expected
        for (slot, expected) in &slots {
            assert_eq!(*slot.get(), *expected);
        }
    }
}

/// Test capacity is never exceeded (separate tests for each capacity)
#[test]
fn bounded_capacity_never_exceeded_1() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(1).build();

    let mut slots = Vec::new();
    for i in 0..200 {
        if let Some(slot) = test_alloc::try_insert(i) {
            slots.push(slot);
        }
    }
    assert!(slots.len() <= 1);
}

#[test]
fn bounded_capacity_never_exceeded_10() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(10).build();

    let mut slots = Vec::new();
    for i in 0..200 {
        if let Some(slot) = test_alloc::try_insert(i) {
            slots.push(slot);
        }
    }
    assert!(slots.len() <= 10);
}

#[test]
fn bounded_capacity_never_exceeded_50() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(50).build();

    let mut slots = Vec::new();
    for i in 0..200 {
        if let Some(slot) = test_alloc::try_insert(i) {
            slots.push(slot);
        }
    }
    assert!(slots.len() <= 50);
}

/// Test fill/drain cycles maintain integrity
#[test]
fn bounded_fill_drain_integrity() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(20).build();

    for cycle in 0..10 {
        // Fill
        let slots: Vec<_> = (0..20)
            .map(|i| test_alloc::insert((cycle * 20 + i) as u64))
            .collect();

        assert_eq!(test_alloc::len(), 20);

        // Verify values
        for (i, slot) in slots.iter().enumerate() {
            assert_eq!(*slot.get(), (cycle * 20 + i) as u64);
        }

        // Drain
        drop(slots);
        assert_eq!(test_alloc::len(), 0);
    }
}

// =============================================================================
// Unbounded Slab Properties
// =============================================================================

#[test]
fn unbounded_len_invariant_random() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(8).build();

    let mut rng = proptest::test_runner::TestRng::deterministic_rng(
        proptest::test_runner::RngAlgorithm::ChaCha
    );

    let mut slots = Vec::new();
    let mut leaked_count = 0usize;

    for _ in 0..500 {
        let action: u8 = rng.random_range(0..10);

        match action {
            0..=5 => {
                slots.push(test_alloc::insert(rng.random()));
            }
            6..=7 => {
                let _ = slots.pop();
            }
            8 => {
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let _ = slots.swap_remove(idx);
                }
            }
            9 => {
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let slot = slots.swap_remove(idx);
                    let _ = slot.leak();
                    leaked_count += 1;
                }
            }
            _ => unreachable!(),
        }

        assert_eq!(test_alloc::len(), slots.len() + leaked_count);
    }
}

#[test]
fn unbounded_value_integrity_random() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(8).build();

    let mut rng = proptest::test_runner::TestRng::deterministic_rng(
        proptest::test_runner::RngAlgorithm::ChaCha
    );

    let mut slots: Vec<(_, u64)> = Vec::new();

    for _ in 0..500 {
        let action: u8 = rng.random_range(0..10);

        match action {
            0..=5 => {
                let value: u64 = rng.random();
                slots.push((test_alloc::insert(value), value));
            }
            6..=7 => {
                let _ = slots.pop();
            }
            8 => {
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let _ = slots.swap_remove(idx);
                }
            }
            9 => {
                if !slots.is_empty() {
                    let idx = rng.random_range(0..slots.len());
                    let new_value: u64 = rng.random();
                    slots[idx].0.replace(new_value);
                    slots[idx].1 = new_value;
                }
            }
            _ => unreachable!(),
        }

        for (slot, expected) in &slots {
            assert_eq!(*slot.get(), *expected);
        }
    }
}

#[test]
fn unbounded_growth_maintains_integrity() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(8).build();

    // Test with increasing counts in a single allocator
    for count in [10, 50, 100, 200] {
        let slots: Vec<_> = (0..count).map(|i| test_alloc::insert(i as u64)).collect();

        for (i, slot) in slots.iter().enumerate() {
            assert_eq!(*slot.get(), i as u64);
        }

        assert!(test_alloc::capacity() >= count);

        // Clean up for next iteration
        drop(slots);
    }
}

#[test]
fn unbounded_cross_chunk_contains_key() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().unbounded().chunk_capacity(8).build();

    let count = 100;
    let slots: Vec<_> = (0..count).map(|i| test_alloc::insert(i as u64)).collect();

    for slot in &slots {
        assert!(test_alloc::contains_key(slot.key()));
    }

    // Leak every 3rd slot, keep the rest
    let mut remaining_slots = Vec::new();
    let mut leaked_keys = Vec::new();

    for (i, slot) in slots.into_iter().enumerate() {
        if i % 3 == 0 {
            leaked_keys.push(slot.leak());
        } else {
            remaining_slots.push(slot);
        }
    }

    for &key in &leaked_keys {
        assert!(test_alloc::contains_key(key));
    }

    for slot in &remaining_slots {
        assert!(test_alloc::contains_key(slot.key()));
    }
}

// =============================================================================
// Freelist Properties
// =============================================================================

#[test]
fn freelist_no_duplicates() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(20).build();

    let mut rng = proptest::test_runner::TestRng::deterministic_rng(
        proptest::test_runner::RngAlgorithm::ChaCha
    );

    let mut slots = Vec::new();
    let mut seen_keys = HashMap::new();

    for _ in 0..200 {
        let should_insert = rng.random_bool(0.6) || slots.is_empty();

        if should_insert {
            if let Some(slot) = test_alloc::try_insert(0) {
                let key = slot.key();
                // Key should not be a duplicate of any currently held slot
                for (existing_key, _) in &slots {
                    assert_ne!(key, *existing_key, "Duplicate key returned!");
                }
                slots.push((key, slot));
                *seen_keys.entry(key.index()).or_insert(0) += 1;
            }
        } else if !slots.is_empty() {
            let idx = rng.random_range(0..slots.len());
            let _ = slots.swap_remove(idx);
        }
    }
}

#[test]
fn freelist_reuses_freed_slots() {
    create_allocator!(test_alloc, u64);
    test_alloc::init().bounded(10).build();

    let mut slots = Vec::new();
    let mut freed_keys = Vec::new();

    for i in 0..100 {
        let should_insert = i % 3 != 2 || slots.is_empty();

        if should_insert {
            if let Some(slot) = test_alloc::try_insert(0) {
                let key = slot.key();
                // If we had freed slots, this should reuse one (LIFO)
                if let Some(expected) = freed_keys.pop() {
                    assert_eq!(key, expected, "Expected LIFO reuse");
                }
                slots.push(slot);
            }
        } else if !slots.is_empty() {
            let slot = slots.pop().unwrap();
            freed_keys.push(slot.key());
        }
    }
}

// =============================================================================
// Drop Counting
// =============================================================================

use std::cell::Cell;

thread_local! {
    static DROP_COUNTER: Cell<usize> = const { Cell::new(0) };
}

pub struct Counted;

impl Drop for Counted {
    fn drop(&mut self) {
        DROP_COUNTER.with(|c| c.set(c.get() + 1));
    }
}

fn reset_counter() {
    DROP_COUNTER.with(|c| c.set(0));
}

fn get_counter() -> usize {
    DROP_COUNTER.with(|c| c.get())
}

#[test]
fn drop_count_matches_inserts() {
    reset_counter();

    create_allocator!(test_alloc, crate::Counted);
    test_alloc::init().bounded(100).build();

    let count = 50;
    {
        let _slots: Vec<_> = (0..count).map(|_| test_alloc::insert(crate::Counted)).collect();
    }

    assert_eq!(get_counter(), count);
}

#[test]
fn into_inner_prevents_drop() {
    reset_counter();

    create_allocator!(test_alloc, crate::Counted);
    test_alloc::init().bounded(100).build();

    let count = 20;
    let slots: Vec<_> = (0..count).map(|_| test_alloc::insert(crate::Counted)).collect();

    // Take half via into_inner
    let half = count / 2;
    let mut values = Vec::new();
    for slot in slots.into_iter().take(half) {
        values.push(slot.into_inner());
    }

    // Rest are dropped, but into_inner ones are not
    assert_eq!(get_counter(), count - half);

    // Now drop the values
    drop(values);
    assert_eq!(get_counter(), count);
}
