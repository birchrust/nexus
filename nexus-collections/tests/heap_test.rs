//! Integration tests for the RcSlot-based pairing heap.

#[allow(dead_code)]
mod pq {
    nexus_collections::heap_allocator!(u64, bounded);
}

fn init() {
    let _ = pq::Allocator::builder().capacity(200).build();
}

// =============================================================================
// Basic operations
// =============================================================================

#[test]
fn empty_heap() {
    init();
    let heap = pq::Heap::new();
    assert!(heap.is_empty());
    assert_eq!(heap.len(), 0);
    assert!(heap.peek().is_none());
}

#[test]
fn push_pop_single() {
    init();
    let mut heap = pq::Heap::new();
    let h = pq::create_node(42).unwrap();

    heap.push(&h);
    assert_eq!(heap.len(), 1);
    assert!(!heap.is_empty());
    assert_eq!(*h.data(), 42);
    assert_eq!(h.strong_count(), 2); // user + heap

    let popped = heap.pop().unwrap();
    assert_eq!(*popped.data(), 42);
    assert!(heap.is_empty());
    assert_eq!(popped.strong_count(), 2); // original handle h + popped (heap's ref transferred)
    drop(h);
    assert_eq!(popped.strong_count(), 1); // just the returned handle
}

#[test]
fn push_pop_sorted_order() {
    init();
    let mut heap = pq::Heap::new();
    let values = [5u64, 3, 8, 1, 7, 2, 6, 4];

    let handles: Vec<_> = values
        .iter()
        .map(|&v| pq::create_node(v).unwrap())
        .collect();

    for h in &handles {
        heap.push(h);
    }
    assert_eq!(heap.len(), 8);

    // Pop should yield sorted order (min-heap)
    let mut result = Vec::new();
    while let Some(h) = heap.pop() {
        result.push(*h.data());
    }
    assert_eq!(result, vec![1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn peek_returns_min() {
    init();
    let mut heap = pq::Heap::new();
    let h5 = pq::create_node(5).unwrap();
    let h3 = pq::create_node(3).unwrap();
    let h7 = pq::create_node(7).unwrap();

    heap.push(&h5);
    assert_eq!(*heap.peek().unwrap().data(), 5);

    heap.push(&h3);
    assert_eq!(*heap.peek().unwrap().data(), 3);

    heap.push(&h7);
    assert_eq!(*heap.peek().unwrap().data(), 3);
}

#[test]
fn pop_empty_returns_none() {
    init();
    let mut heap = pq::Heap::new();
    assert!(heap.pop().is_none());
    assert!(heap.pop().is_none());
}

// =============================================================================
// Contains
// =============================================================================

#[test]
fn contains_linked_node() {
    init();
    let mut heap = pq::Heap::new();
    let h = pq::create_node(1).unwrap();

    assert!(!heap.contains(&h));
    heap.push(&h);
    assert!(heap.contains(&h));
    heap.pop();
    assert!(!heap.contains(&h));
}

#[test]
fn contains_multiple_nodes() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(1).unwrap();
    let h2 = pq::create_node(2).unwrap();
    let h3 = pq::create_node(3).unwrap();

    heap.push(&h1);
    heap.push(&h2);
    heap.push(&h3);

    assert!(heap.contains(&h1));
    assert!(heap.contains(&h2));
    assert!(heap.contains(&h3));

    heap.unlink(&h2);
    assert!(!heap.contains(&h2));
    assert!(heap.contains(&h1));
    assert!(heap.contains(&h3));
}

// =============================================================================
// Unlink
// =============================================================================

#[test]
fn unlink_root() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(1).unwrap();
    let h3 = pq::create_node(3).unwrap();
    let h5 = pq::create_node(5).unwrap();

    heap.push(&h1);
    heap.push(&h3);
    heap.push(&h5);

    heap.unlink(&h1); // remove root (min)
    assert_eq!(heap.len(), 2);
    assert_eq!(h1.strong_count(), 1); // only user handle

    // Remaining should pop in order
    assert_eq!(*heap.pop().unwrap().data(), 3);
    assert_eq!(*heap.pop().unwrap().data(), 5);
}

#[test]
fn unlink_non_root() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(1).unwrap();
    let h3 = pq::create_node(3).unwrap();
    let h5 = pq::create_node(5).unwrap();
    let h7 = pq::create_node(7).unwrap();

    heap.push(&h1);
    heap.push(&h3);
    heap.push(&h5);
    heap.push(&h7);

    heap.unlink(&h5); // remove non-root node
    assert_eq!(heap.len(), 3);

    let mut result = Vec::new();
    while let Some(h) = heap.pop() {
        result.push(*h.data());
    }
    assert_eq!(result, vec![1, 3, 7]);
}

#[test]
fn unlink_sole_element() {
    init();
    let mut heap = pq::Heap::new();
    let h = pq::create_node(42).unwrap();

    heap.push(&h);
    heap.unlink(&h);
    assert!(heap.is_empty());
    assert_eq!(h.strong_count(), 1);
}

#[test]
fn push_unlink_repush() {
    init();
    let mut heap = pq::Heap::new();
    let h = pq::create_node(10).unwrap();

    heap.push(&h);
    heap.unlink(&h);
    assert_eq!(h.strong_count(), 1); // heap released its ref

    heap.push(&h);
    assert!(heap.contains(&h));
    assert_eq!(*heap.pop().unwrap().data(), 10);
}

#[test]
fn unlink_node_with_children() {
    init();
    let mut heap = pq::Heap::new();
    // Push in order that guarantees h3 has children:
    // push 3 (root=3), push 5 (5 child of 3), push 7 (7 child of 3),
    // push 1 (root=1, 3 child of 1). Now 3 has children 5 and 7.
    let h1 = pq::create_node(1).unwrap();
    let h3 = pq::create_node(3).unwrap();
    let h5 = pq::create_node(5).unwrap();
    let h7 = pq::create_node(7).unwrap();

    heap.push(&h3);
    heap.push(&h5);
    heap.push(&h7);
    heap.push(&h1);

    // Unlink h3 — its children (5, 7) should be merged back
    heap.unlink(&h3);
    assert_eq!(heap.len(), 3);

    let mut result = Vec::new();
    while let Some(h) = heap.pop() {
        result.push(*h.data());
    }
    assert_eq!(result, vec![1, 5, 7]);
}

// =============================================================================
// Clear and Drop
// =============================================================================

#[test]
fn clear_releases_all_refs() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(1).unwrap();
    let h2 = pq::create_node(2).unwrap();
    let h3 = pq::create_node(3).unwrap();

    heap.push(&h1);
    heap.push(&h2);
    heap.push(&h3);

    assert_eq!(h1.strong_count(), 2);
    assert_eq!(h2.strong_count(), 2);
    assert_eq!(h3.strong_count(), 2);

    heap.clear();

    assert!(heap.is_empty());
    assert_eq!(h1.strong_count(), 1);
    assert_eq!(h2.strong_count(), 1);
    assert_eq!(h3.strong_count(), 1);
}

#[test]
fn drop_heap_releases_refs() {
    init();
    let h1 = pq::create_node(1).unwrap();
    let h2 = pq::create_node(2).unwrap();

    {
        let mut heap = pq::Heap::new();
        heap.push(&h1);
        heap.push(&h2);
        assert_eq!(h1.strong_count(), 2);
    } // heap drops here

    assert_eq!(h1.strong_count(), 1);
    assert_eq!(h2.strong_count(), 1);
}

// =============================================================================
// Drain
// =============================================================================

#[test]
fn drain_sorted_order() {
    init();
    let mut heap = pq::Heap::new();
    let values = [5u64, 1, 3, 7, 2];
    let handles: Vec<_> = values
        .iter()
        .map(|&v| pq::create_node(v).unwrap())
        .collect();

    for h in &handles {
        heap.push(h);
    }

    let result: Vec<u64> = heap.drain().map(|h| *h.data()).collect();
    assert_eq!(result, vec![1, 2, 3, 5, 7]);
    assert!(heap.is_empty());
}

#[test]
fn drain_partial_then_drop_clears() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(1).unwrap();
    let h2 = pq::create_node(2).unwrap();
    let h3 = pq::create_node(3).unwrap();

    heap.push(&h1);
    heap.push(&h2);
    heap.push(&h3);

    {
        let mut drain = heap.drain();
        let first = drain.next().unwrap();
        assert_eq!(*first.data(), 1);
        // drop drain — remaining elements cleared
    }

    assert!(heap.is_empty());
    // h2 and h3 had their heap refs released by clear
    assert_eq!(h2.strong_count(), 1);
    assert_eq!(h3.strong_count(), 1);
}

#[test]
fn drain_exact_size() {
    init();
    let mut heap = pq::Heap::new();
    for v in [3u64, 1, 2] {
        let h = pq::create_node(v).unwrap();
        heap.push(&h);
    }

    let drain = heap.drain();
    assert_eq!(drain.len(), 3);
}

// =============================================================================
// DrainWhile
// =============================================================================

#[test]
fn drain_while_partial() {
    init();
    let mut heap = pq::Heap::new();
    let values = [1u64, 3, 5, 7, 9];
    let handles: Vec<_> = values
        .iter()
        .map(|&v| pq::create_node(v).unwrap())
        .collect();

    for h in &handles {
        heap.push(h);
    }

    // Drain while value < 5
    let drained: Vec<u64> = heap
        .drain_while(|node| *node.data() < 5)
        .map(|h| *h.data())
        .collect();

    assert_eq!(drained, vec![1, 3]);
    assert_eq!(heap.len(), 3); // 5, 7, 9 remain
}

#[test]
fn drain_while_all() {
    init();
    let mut heap = pq::Heap::new();
    for v in [1u64, 2, 3] {
        let h = pq::create_node(v).unwrap();
        heap.push(&h);
    }

    let drained: Vec<u64> = heap
        .drain_while(|node| *node.data() < 100)
        .map(|h| *h.data())
        .collect();

    assert_eq!(drained, vec![1, 2, 3]);
    assert!(heap.is_empty());
}

#[test]
fn drain_while_none() {
    init();
    let mut heap = pq::Heap::new();
    for v in [10u64, 20, 30] {
        let h = pq::create_node(v).unwrap();
        heap.push(&h);
    }

    let drained: Vec<u64> = heap
        .drain_while(|node| *node.data() < 5)
        .map(|h| *h.data())
        .collect();

    assert!(drained.is_empty());
    assert_eq!(heap.len(), 3); // all remain
}

#[test]
fn drain_while_empty_heap() {
    init();
    let mut heap = pq::Heap::new();
    let drained: Vec<u64> = heap.drain_while(|_| true).map(|h| *h.data()).collect();
    assert!(drained.is_empty());
}

// =============================================================================
// Large heap
// =============================================================================

#[test]
fn large_heap_sorted_drain() {
    init();
    let mut heap = pq::Heap::new();
    let n = 127u64; // full binary tree size

    let handles: Vec<_> = (0..n).rev().map(|v| pq::create_node(v).unwrap()).collect();

    for h in &handles {
        heap.push(h);
    }
    assert_eq!(heap.len(), n as usize);

    let result: Vec<u64> = heap.drain().map(|h| *h.data()).collect();
    let expected: Vec<u64> = (0..n).collect();
    assert_eq!(result, expected);
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn pop_two_elements() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(2).unwrap();
    let h2 = pq::create_node(1).unwrap();

    heap.push(&h1);
    heap.push(&h2);

    assert_eq!(*heap.pop().unwrap().data(), 1);
    assert_eq!(*heap.pop().unwrap().data(), 2);
    assert!(heap.is_empty());
}

#[test]
fn duplicate_values() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(5).unwrap();
    let h2 = pq::create_node(5).unwrap();
    let h3 = pq::create_node(5).unwrap();

    heap.push(&h1);
    heap.push(&h2);
    heap.push(&h3);

    assert_eq!(*heap.pop().unwrap().data(), 5);
    assert_eq!(*heap.pop().unwrap().data(), 5);
    assert_eq!(*heap.pop().unwrap().data(), 5);
    assert!(heap.is_empty());
}

#[test]
fn already_sorted_input() {
    init();
    let mut heap = pq::Heap::new();

    let handles: Vec<_> = (1..=10u64).map(|v| pq::create_node(v).unwrap()).collect();

    for h in &handles {
        heap.push(h);
    }

    let result: Vec<u64> = heap.drain().map(|h| *h.data()).collect();
    let expected: Vec<u64> = (1..=10).collect();
    assert_eq!(result, expected);
}

#[test]
fn reverse_sorted_input() {
    init();
    let mut heap = pq::Heap::new();

    let handles: Vec<_> = (1..=10u64)
        .rev()
        .map(|v| pq::create_node(v).unwrap())
        .collect();

    for h in &handles {
        heap.push(h);
    }

    let result: Vec<u64> = heap.drain().map(|h| *h.data()).collect();
    let expected: Vec<u64> = (1..=10).collect();
    assert_eq!(result, expected);
}

#[test]
fn unlink_all_then_repush() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(3).unwrap();
    let h2 = pq::create_node(1).unwrap();
    let h3 = pq::create_node(2).unwrap();

    heap.push(&h1);
    heap.push(&h2);
    heap.push(&h3);

    heap.unlink(&h1);
    heap.unlink(&h2);
    heap.unlink(&h3);
    assert!(heap.is_empty());

    // Re-push in different order
    heap.push(&h3);
    heap.push(&h1);
    heap.push(&h2);

    let result: Vec<u64> = heap.drain().map(|h| *h.data()).collect();
    assert_eq!(result, vec![1, 2, 3]);
}

// =============================================================================
// Debug-only panics
// =============================================================================

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "already linked")]
fn push_already_linked_panics() {
    init();
    let mut heap = pq::Heap::new();
    let h1 = pq::create_node(1).unwrap();
    let h2 = pq::create_node(2).unwrap();
    // Push two elements so h2 has a parent pointer (not root)
    heap.push(&h1);
    heap.push(&h2);
    heap.push(&h2); // panic — h2 has non-null pointers
}

// =============================================================================
// Stress: interleaved push/pop/unlink
// =============================================================================

#[test]
fn interleaved_operations() {
    init();
    let mut heap = pq::Heap::new();

    let h1 = pq::create_node(10).unwrap();
    let h2 = pq::create_node(20).unwrap();
    let h3 = pq::create_node(5).unwrap();
    let h4 = pq::create_node(15).unwrap();

    heap.push(&h1); // [10]
    heap.push(&h2); // [10, 20]
    heap.push(&h3); // [5, 10, 20]

    assert_eq!(*heap.pop().unwrap().data(), 5); // [10, 20]

    heap.push(&h4); // [10, 15, 20]
    heap.unlink(&h2); // [10, 15]

    assert_eq!(*heap.pop().unwrap().data(), 10); // [15]
    assert_eq!(*heap.pop().unwrap().data(), 15); // []
    assert!(heap.is_empty());
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn pop_after_user_drops_handle() {
    init();
    let mut heap = pq::Heap::new();
    let h = pq::create_node(42).unwrap();
    heap.push(&h);
    assert_eq!(h.strong_count(), 2); // user + heap

    drop(h); // strong = 1, only heap holds ref

    let popped = heap.pop().unwrap();
    assert_eq!(*popped.data(), 42);
    assert_eq!(popped.strong_count(), 1); // sole owner
}

#[test]
fn two_heaps_same_allocator() {
    init();
    let mut heap_a = pq::Heap::new();
    let mut heap_b = pq::Heap::new();

    let h1 = pq::create_node(10).unwrap();
    let h2 = pq::create_node(20).unwrap();
    let h3 = pq::create_node(1).unwrap();
    let h4 = pq::create_node(5).unwrap();

    heap_a.push(&h1);
    heap_a.push(&h2);
    heap_b.push(&h3);
    heap_b.push(&h4);

    assert_eq!(heap_a.len(), 2);
    assert_eq!(heap_b.len(), 2);

    assert_eq!(*heap_a.pop().unwrap().data(), 10);
    assert_eq!(*heap_b.pop().unwrap().data(), 1);
    assert_eq!(*heap_a.pop().unwrap().data(), 20);
    assert_eq!(*heap_b.pop().unwrap().data(), 5);
}

#[test]
fn drain_empty_heap() {
    init();
    let mut heap = pq::Heap::new();
    let result: Vec<u64> = heap.drain().map(|h| *h.data()).collect();
    assert!(result.is_empty());
}

#[test]
fn default_construction() {
    init();
    let heap = pq::Heap::default();
    assert!(heap.is_empty());
    assert_eq!(heap.len(), 0);
}

// =============================================================================
// Unbounded allocator variant
// =============================================================================

#[allow(dead_code)]
mod pq_unbounded {
    nexus_collections::heap_allocator!(u64, unbounded);
}

fn init_unbounded() {
    let _ = pq_unbounded::Allocator::builder().build();
}

#[test]
fn unbounded_push_pop() {
    init_unbounded();
    let mut heap = pq_unbounded::Heap::new();

    let h1 = pq_unbounded::create_node(5);
    let h2 = pq_unbounded::create_node(3);
    let h3 = pq_unbounded::create_node(7);

    heap.push(&h1);
    heap.push(&h2);
    heap.push(&h3);

    assert_eq!(*heap.pop().unwrap().data(), 3);
    assert_eq!(*heap.pop().unwrap().data(), 5);
    assert_eq!(*heap.pop().unwrap().data(), 7);
}

#[test]
fn unbounded_drain() {
    init_unbounded();
    let mut heap = pq_unbounded::Heap::new();

    for v in [10, 1, 8, 3, 5] {
        let h = pq_unbounded::create_node(v);
        heap.push(&h);
    }

    let result: Vec<u64> = heap.drain().map(|h| *h.data()).collect();
    assert_eq!(result, vec![1, 3, 5, 8, 10]);
}

