//! Integration tests for the v0.8.0 RcSlot-based list.

use nexus_collections::list_allocator;

#[derive(Debug)]
pub struct Order {
    id: u64,
    price: f64,
}

#[allow(dead_code)]
mod orders {
    use super::*;
    list_allocator!(Order, bounded);
}

fn init() {
    let _ = orders::Allocator::builder().capacity(100).build();
}

// =============================================================================
// Basic operations
// =============================================================================

#[test]
fn empty_list() {
    init();
    let list = orders::List::new();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    assert!(list.front().is_none());
    assert!(list.back().is_none());
}

#[test]
fn link_back_single() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    assert_eq!(list.len(), 1);
    assert!(!list.is_empty());
    assert_eq!(h.exclusive().id, 1);
    assert_eq!(h.strong_count(), 2); // user + list
}

#[test]
fn link_front_single() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_front(&h);
    assert_eq!(list.len(), 1);
    assert_eq!(h.strong_count(), 2);
}

#[test]
fn link_back_multiple() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    assert_eq!(list.len(), 3);
    assert_eq!(list.front().unwrap().exclusive().id, 1);
    assert_eq!(list.back().unwrap().exclusive().id, 3);
}

#[test]
fn link_front_multiple() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list.link_back(&h1);
    list.link_front(&h2);

    // h2 is now front, h1 is back
    assert_eq!(list.front().unwrap().exclusive().id, 2);
    assert_eq!(list.back().unwrap().exclusive().id, 1);
}

// =============================================================================
// Unlink
// =============================================================================

#[test]
fn unlink_single() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    assert_eq!(h.strong_count(), 2);

    list.unlink(&h);
    assert_eq!(list.len(), 0);
    assert!(list.is_empty());
    assert_eq!(h.strong_count(), 1);
    assert!(!h.is_linked());

    // Handle still accessible
    assert_eq!(h.exclusive().id, 1);
}

#[test]
fn unlink_middle() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    list.unlink(&h2);
    assert_eq!(list.len(), 2);
    assert_eq!(list.front().unwrap().exclusive().id, 1);
    assert_eq!(list.back().unwrap().exclusive().id, 3);
}

#[test]
fn unlink_and_relink() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    list.unlink(&h);
    list.link_front(&h);

    assert_eq!(list.len(), 1);
    assert_eq!(h.strong_count(), 2);
}

// =============================================================================
// Pop
// =============================================================================

#[test]
fn pop_front_single() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    let popped = list.pop_front().unwrap();
    assert_eq!(popped.exclusive().id, 1);
    assert!(list.is_empty());
    // popped carries the list's strong ref — user handle + popped = 2
    // (user handle is h, popped is the list's transferred ref)
    assert_eq!(h.strong_count(), 2);
    drop(popped);
    assert_eq!(h.strong_count(), 1);
}

#[test]
fn pop_back_single() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    let popped = list.pop_back().unwrap();
    assert_eq!(popped.exclusive().id, 1);
    assert!(list.is_empty());
}

#[test]
fn pop_front_multiple() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    let p1 = list.pop_front().unwrap();
    assert_eq!(p1.exclusive().id, 1);
    assert_eq!(list.len(), 2);

    let p2 = list.pop_front().unwrap();
    assert_eq!(p2.exclusive().id, 2);

    let p3 = list.pop_front().unwrap();
    assert_eq!(p3.exclusive().id, 3);

    assert!(list.pop_front().is_none());
}

#[test]
fn pop_empty() {
    init();
    let mut list = orders::List::new();
    assert!(list.pop_front().is_none());
    assert!(list.pop_back().is_none());
}

// =============================================================================
// Position checks
// =============================================================================

#[test]
fn is_head_and_tail() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);

    assert!(list.is_head(&h1));
    assert!(!list.is_head(&h2));
    assert!(!list.is_tail(&h1));
    assert!(list.is_tail(&h2));
}

// =============================================================================
// Relative insertion
// =============================================================================

#[test]
fn link_after() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h3);
    list.link_after(&h1, &h2); // [1, 2, 3]

    assert_eq!(list.len(), 3);
    assert_eq!(list.front().unwrap().exclusive().id, 1);
    assert_eq!(list.back().unwrap().exclusive().id, 3);

    // Verify order via pop
    let p1 = list.pop_front().unwrap();
    assert_eq!(p1.exclusive().id, 1);
    let p2 = list.pop_front().unwrap();
    assert_eq!(p2.exclusive().id, 2);
    let p3 = list.pop_front().unwrap();
    assert_eq!(p3.exclusive().id, 3);
}

#[test]
fn link_after_at_tail() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list.link_back(&h1);
    list.link_after(&h1, &h2); // [1, 2]

    assert!(list.is_tail(&h2));
    assert_eq!(list.back().unwrap().exclusive().id, 2);
}

#[test]
fn link_before() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h3);
    list.link_before(&h3, &h2); // [1, 2, 3]

    let p1 = list.pop_front().unwrap();
    assert_eq!(p1.exclusive().id, 1);
    let p2 = list.pop_front().unwrap();
    assert_eq!(p2.exclusive().id, 2);
}

#[test]
fn link_before_at_head() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list.link_back(&h1);
    list.link_before(&h1, &h2); // [2, 1]

    assert!(list.is_head(&h2));
    assert_eq!(list.front().unwrap().exclusive().id, 2);
}

// =============================================================================
// Move operations
// =============================================================================

#[test]
fn move_to_front() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    list.move_to_front(&h3); // [3, 1, 2]

    assert_eq!(list.front().unwrap().exclusive().id, 3);
    assert_eq!(list.back().unwrap().exclusive().id, 2);
}

#[test]
fn move_to_front_already_front() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);

    list.move_to_front(&h1); // no-op
    assert_eq!(list.front().unwrap().exclusive().id, 1);
}

#[test]
fn move_to_back() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    list.move_to_back(&h1); // [2, 3, 1]

    assert_eq!(list.front().unwrap().exclusive().id, 2);
    assert_eq!(list.back().unwrap().exclusive().id, 1);
}

// =============================================================================
// Data access via ExclusiveCell
// =============================================================================

#[test]
fn write_via_exclusive_mut() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    h.exclusive_mut().price = 99.0;
    assert_eq!(h.exclusive().price, 99.0);
}

#[test]
fn peek_front_write() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    list.front().unwrap().exclusive_mut().price = 42.0;
    assert_eq!(h.exclusive().price, 42.0);
}

#[test]
#[should_panic(expected = "already borrowed")]
fn double_borrow_panics() {
    init();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let _r1 = h.exclusive();
    let _r2 = h.exclusive(); // panic
}

// =============================================================================
// Refcount semantics
// =============================================================================

#[test]
fn strong_count_lifecycle() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    assert_eq!(h.strong_count(), 1);

    list.link_back(&h);
    assert_eq!(h.strong_count(), 2);

    list.unlink(&h);
    assert_eq!(h.strong_count(), 1);

    list.link_back(&h);
    assert_eq!(h.strong_count(), 2);

    // Drop list while linked
    drop(list);
    assert_eq!(h.strong_count(), 1);
    // Handle still valid
    assert_eq!(h.exclusive().id, 1);
}

#[test]
fn drop_handle_while_linked() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    assert_eq!(h.strong_count(), 2);

    drop(h);
    // List still valid, node alive via list's ref
    assert_eq!(list.len(), 1);
    assert_eq!(list.front().unwrap().exclusive().id, 1);

    // Pop returns the list's ref — now strong = 1
    let popped = list.pop_front().unwrap();
    assert_eq!(popped.strong_count(), 1);
    assert_eq!(popped.exclusive().id, 1);
}

// =============================================================================
// Multiple lists sharing allocator
// =============================================================================

#[test]
fn two_lists_same_allocator() {
    init();
    let mut list_a = orders::List::new();
    let mut list_b = orders::List::new();

    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list_a.link_back(&h1);
    list_b.link_back(&h2);

    assert_eq!(list_a.len(), 1);
    assert_eq!(list_b.len(), 1);
}

#[test]
fn move_between_lists() {
    init();
    let mut list_a = orders::List::new();
    let mut list_b = orders::List::new();

    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list_a.link_back(&h);
    assert_eq!(h.strong_count(), 2);

    list_a.unlink(&h);
    assert_eq!(h.strong_count(), 1);

    list_b.link_back(&h);
    assert_eq!(h.strong_count(), 2);
    assert_eq!(list_b.front().unwrap().exclusive().id, 1);
}

// =============================================================================
// List ID uniqueness
// =============================================================================

#[test]
fn list_ids_unique() {
    init();
    let list_a = orders::List::new();
    let list_b = orders::List::new();
    assert_ne!(list_a.id(), list_b.id());
}

#[test]
#[should_panic(expected = "node does not belong to this list")]
fn cross_list_unlink_panics() {
    init();
    let mut list_a = orders::List::new();
    let mut list_b = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list_a.link_back(&h);
    list_b.unlink(&h); // panic
}

#[test]
#[should_panic(expected = "node does not belong to this list")]
fn unlink_detached_panics() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.unlink(&h); // panic — not linked
}

#[test]
#[should_panic(expected = "node is already linked")]
fn double_link_panics() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    list.link_back(&h); // panic
}

// =============================================================================
// contains
// =============================================================================

#[test]
fn contains_linked_node() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    assert!(!list.contains(&h));
    list.link_back(&h);
    assert!(list.contains(&h));
    list.unlink(&h);
    assert!(!list.contains(&h));
}

#[test]
fn contains_wrong_list() {
    init();
    let mut list_a = orders::List::new();
    let list_b = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list_a.link_back(&h);
    assert!(list_a.contains(&h));
    assert!(!list_b.contains(&h));
}

// =============================================================================
// clear
// =============================================================================

#[test]
fn clear_empty() {
    init();
    let mut list = orders::List::new();
    list.clear();
    assert!(list.is_empty());
}

#[test]
fn clear_releases_strong_refs() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);
    assert_eq!(h1.strong_count(), 2);

    list.clear();

    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    assert!(list.front().is_none());
    assert!(list.back().is_none());

    // Handles still valid, back to strong=1
    assert_eq!(h1.strong_count(), 1);
    assert_eq!(h1.exclusive().id, 1);
    assert!(!h1.is_linked());
}

#[test]
fn clear_then_reuse() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list.link_back(&h1);
    list.clear();

    // Can re-link after clear
    list.link_back(&h2);
    assert_eq!(list.len(), 1);
    assert_eq!(list.front().unwrap().exclusive().id, 2);
}

// =============================================================================
// Cursor iteration after user drops handle (strong=1 during traversal)
// =============================================================================

#[test]
fn cursor_iterate_after_user_drops_handle() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    // Drop user handles — list is sole owner (strong=1)
    drop(h1);
    drop(h2);
    drop(h3);

    // Cursor iteration should still work via list's strong ref
    let mut cursor = list.cursor();
    let mut ids = Vec::new();
    while cursor.advance() {
        ids.push(cursor.current().unwrap().exclusive().id);
    }
    assert_eq!(ids, vec![1, 2, 3]);
    assert_eq!(list.len(), 3);
}

// =============================================================================
// Pop after user drops handle (strong=1, sole owner via from_raw)
// =============================================================================

#[test]
fn pop_after_user_drops_handle() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);
    assert_eq!(h.strong_count(), 2);

    // Drop user handle — list is sole owner
    drop(h);

    // Pop transfers list's strong ref to the caller
    let popped = list.pop_front().unwrap();
    assert_eq!(popped.strong_count(), 1);
    assert_eq!(popped.exclusive().id, 1);
    assert!(list.is_empty());
}

// =============================================================================
// Cross-list link_after / link_before panics
// =============================================================================

#[test]
#[should_panic(expected = "anchor node does not belong to this list")]
fn cross_list_link_after_panics() {
    init();
    let mut list_a = orders::List::new();
    let mut list_b = orders::List::new();
    let anchor = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let new_node = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list_a.link_back(&anchor);
    // anchor belongs to list_a, but we call link_after on list_b
    list_b.link_after(&anchor, &new_node); // panic
}

#[test]
#[should_panic(expected = "anchor node does not belong to this list")]
fn cross_list_link_before_panics() {
    init();
    let mut list_a = orders::List::new();
    let mut list_b = orders::List::new();
    let anchor = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let new_node = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list_a.link_back(&anchor);
    // anchor belongs to list_a, but we call link_before on list_b
    list_b.link_before(&anchor, &new_node); // panic
}
