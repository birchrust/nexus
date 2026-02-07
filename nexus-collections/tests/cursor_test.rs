//! Integration tests for cursor operations (v0.8.0 split API).

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
// Basic iteration
// =============================================================================

#[test]
fn cursor_forward_iteration() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    let mut cursor = list.cursor();
    let mut ids = Vec::new();
    while cursor.advance() {
        ids.push(cursor.current().unwrap().exclusive().id);
    }
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn cursor_backward_iteration() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    let mut cursor = list.cursor_back();
    let mut ids = Vec::new();
    while cursor.advance_back() {
        ids.push(cursor.current().unwrap().exclusive().id);
    }
    assert_eq!(ids, vec![3, 2, 1]);
}

#[test]
fn cursor_empty_list() {
    init();
    let mut list = orders::List::new();
    let mut cursor = list.cursor();
    assert!(!cursor.advance());
    assert!(cursor.current().is_none());
}

// =============================================================================
// Removal (std-style: advance to position, check current, remove or advance)
// =============================================================================

#[test]
fn cursor_remove_middle() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    let mut cursor = list.cursor();
    cursor.advance(); // at 1
    loop {
        match cursor.current() {
            None => break,
            Some(node) if node.exclusive().id == 2 => {
                let removed = cursor.remove(); // auto-advances to 3
                assert_eq!(removed.exclusive().id, 2);
            }
            Some(_) => {
                cursor.advance();
            }
        }
    }

    assert_eq!(list.len(), 2);
    assert_eq!(list.front().unwrap().exclusive().id, 1);
    assert_eq!(list.back().unwrap().exclusive().id, 3);
}

#[test]
fn cursor_remove_head() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);

    let mut cursor = list.cursor();
    cursor.advance(); // at 1
    let removed = cursor.remove(); // auto-advances to 2
    assert_eq!(removed.exclusive().id, 1);

    assert_eq!(list.len(), 1);
    assert_eq!(list.front().unwrap().exclusive().id, 2);
}

#[test]
fn cursor_remove_tail() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);

    let mut cursor = list.cursor_back();
    cursor.advance_back(); // at 2
    let removed = cursor.remove(); // tail removed, cursor at AfterEnd
    assert_eq!(removed.exclusive().id, 2);
    assert!(cursor.current().is_none());
    drop(cursor);

    assert_eq!(list.len(), 1);
    assert_eq!(list.back().unwrap().exclusive().id, 1);
}

#[test]
fn cursor_remove_all() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    let mut removed = Vec::new();
    let mut cursor = list.cursor();
    cursor.advance(); // at first node
    loop {
        match cursor.current() {
            None => break,
            Some(_) => {
                removed.push(cursor.remove()); // auto-advances
            }
        }
    }

    assert!(list.is_empty());
    assert_eq!(removed.len(), 3);
}

// =============================================================================
// Auto-advance after removal
// =============================================================================

#[test]
fn cursor_continues_after_remove() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    let mut ids_after_remove = Vec::new();
    let mut cursor = list.cursor();
    cursor.advance(); // at 1
    cursor.advance(); // at 2

    // Remove 2, cursor auto-advances to 3
    let _removed = cursor.remove();

    // current() should now be at 3
    assert_eq!(cursor.current().unwrap().exclusive().id, 3);

    // Collect remaining via loop
    loop {
        match cursor.current() {
            None => break,
            Some(node) => {
                ids_after_remove.push(node.exclusive().id);
                cursor.advance();
            }
        }
    }

    assert_eq!(ids_after_remove, vec![3]);
}

#[test]
fn cursor_backward_after_remove() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    let mut cursor = list.cursor();
    cursor.advance(); // at 1
    cursor.advance(); // at 2

    // Remove 2, cursor auto-advances to 3
    let _removed = cursor.remove();
    assert_eq!(cursor.current().unwrap().exclusive().id, 3);

    // Go backward from 3 — should get 1
    cursor.advance_back();
    assert_eq!(cursor.current().unwrap().exclusive().id, 1);
}

// =============================================================================
// Conditional removal
// =============================================================================

#[test]
fn cursor_remove_if_matches() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);

    let mut removed_ids = Vec::new();
    let mut cursor = list.cursor();
    cursor.advance(); // at first node
    loop {
        match cursor.current() {
            None => break,
            Some(_) => {
                if let Some(removed) = cursor.remove_if(|n| n.exclusive().price > 15.0) {
                    removed_ids.push(removed.exclusive().id);
                } else {
                    cursor.advance();
                }
            }
        }
    }

    assert_eq!(removed_ids, vec![2, 3]);
    assert_eq!(list.len(), 1);
    assert_eq!(list.front().unwrap().exclusive().id, 1);
}

#[test]
fn cursor_remove_if_no_match() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h1);

    let mut cursor = list.cursor();
    cursor.advance(); // at 1
    let result = cursor.remove_if(|_| false);
    assert!(result.is_none());
    // cursor should still be at 1
    assert_eq!(cursor.current().unwrap().exclusive().id, 1);

    drop(cursor);
    assert_eq!(list.len(), 1);
}

// =============================================================================
// Single element
// =============================================================================

#[test]
fn cursor_single_element() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);

    let mut cursor = list.cursor();
    assert!(cursor.advance());
    assert_eq!(cursor.current().unwrap().exclusive().id, 1);
    assert!(!cursor.advance()); // past end
    assert!(cursor.current().is_none());
}

#[test]
fn cursor_remove_single_element() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);

    let mut cursor = list.cursor();
    cursor.advance(); // at 1
    let _removed = cursor.remove(); // auto-advances to AfterEnd
    assert!(cursor.current().is_none());
    drop(cursor);

    assert!(list.is_empty());
}

// =============================================================================
// Multiple consecutive removals
// =============================================================================

#[test]
fn cursor_consecutive_removals() {
    init();
    let mut list = orders::List::new();
    let h1 = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    let h2 = orders::create_node(Order { id: 2, price: 20.0 }).unwrap();
    let h3 = orders::create_node(Order { id: 3, price: 30.0 }).unwrap();
    let h4 = orders::create_node(Order { id: 4, price: 40.0 }).unwrap();

    list.link_back(&h1);
    list.link_back(&h2);
    list.link_back(&h3);
    list.link_back(&h4);

    // Remove 2 and 3 consecutively
    let mut cursor = list.cursor();
    cursor.advance(); // at 1
    cursor.advance(); // at 2

    let _r1 = cursor.remove(); // remove 2, auto-advance to 3
    assert_eq!(cursor.current().unwrap().exclusive().id, 3);

    let _r2 = cursor.remove(); // remove 3, auto-advance to 4
    assert_eq!(cursor.current().unwrap().exclusive().id, 4);

    drop(cursor);

    assert_eq!(list.len(), 2);
    assert_eq!(list.front().unwrap().exclusive().id, 1);
    assert_eq!(list.back().unwrap().exclusive().id, 4);
}

// =============================================================================
// Write via cursor
// =============================================================================

#[test]
fn cursor_write_via_guard() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();

    list.link_back(&h);

    let mut cursor = list.cursor();
    cursor.advance(); // at 1
    cursor.current().unwrap().exclusive_mut().price = 999.0;
    drop(cursor);

    assert_eq!(h.exclusive().price, 999.0);
}

// =============================================================================
// Panic paths
// =============================================================================

#[test]
#[should_panic(expected = "cursor is not positioned at a node")]
fn cursor_remove_panics_when_not_at_node() {
    init();
    let mut list = orders::List::new();
    let mut cursor = list.cursor();
    let _ = cursor.remove(); // BeforeStart → panic
}

#[test]
#[should_panic(expected = "cursor is not positioned at a node")]
fn cursor_remove_if_panics_when_not_at_node() {
    init();
    let mut list = orders::List::new();
    let h = orders::create_node(Order { id: 1, price: 10.0 }).unwrap();
    list.link_back(&h);
    let mut cursor = list.cursor();
    // Don't advance — still at BeforeStart
    cursor.remove_if(|_| true); // panic
}
