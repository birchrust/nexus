//! Integration tests for the RcSlot-based list.

use nexus_collections::list::{List, ListNode};
use nexus_slab::rc::bounded::Slab;

#[derive(Debug)]
#[allow(dead_code)]
struct Order {
    id: u64,
    price: f64,
}

fn make_slab() -> Slab<ListNode<Order>> {
    unsafe { Slab::with_capacity(100) }
}

fn make_u64_slab() -> Slab<ListNode<u64>> {
    unsafe { Slab::with_capacity(100) }
}

// =============================================================================
// Basic operations
// =============================================================================

#[test]
fn empty_list() {
    let mut list = List::<Order>::new();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    assert!(list.front().is_none());
    assert!(list.back().is_none());
    let slab = make_slab();
    list.clear(&slab);
}

#[test]
fn link_back_single() {
    let slab = make_slab();
    let mut list = List::new();
    let h = slab.alloc(ListNode::new(Order { id: 1, price: 10.0 }));

    list.link_back(&h);
    assert_eq!(list.len(), 1);
    assert!(!list.is_empty());
    assert_eq!(h.borrow().value.id, 1);
    assert_eq!(h.refcount(), 2); // user + list

    list.clear(&slab);
    slab.free(h);
}

#[test]
fn link_front_single() {
    let slab = make_slab();
    let mut list = List::new();
    let h = slab.alloc(ListNode::new(Order { id: 2, price: 20.0 }));

    list.link_front(&h);
    assert_eq!(list.len(), 1);

    list.clear(&slab);
    slab.free(h);
}

#[test]
fn push_back_and_pop_front() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h2 = list.try_push_back(&slab, 20).unwrap();
    let h3 = list.try_push_back(&slab, 30).unwrap();

    assert_eq!(list.len(), 3);
    assert_eq!(list.front().unwrap().value, 10);
    assert_eq!(list.back().unwrap().value, 30);

    let p1 = list.pop_front().unwrap();
    assert_eq!(p1.borrow().value, 10);
    assert_eq!(list.len(), 2);

    let p2 = list.pop_front().unwrap();
    assert_eq!(p2.borrow().value, 20);

    let p3 = list.pop_front().unwrap();
    assert_eq!(p3.borrow().value, 30);

    assert!(list.is_empty());
    assert!(list.pop_front().is_none());

    // Free all handles
    slab.free(h1);
    slab.free(h2);
    slab.free(h3);
    slab.free(p1);
    slab.free(p2);
    slab.free(p3);
}

#[test]
fn push_front_and_pop_back() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_front(&slab, 10).unwrap();
    let h2 = list.try_push_front(&slab, 20).unwrap();

    assert_eq!(list.front().unwrap().value, 20);
    assert_eq!(list.back().unwrap().value, 10);

    let p = list.pop_back().unwrap();
    assert_eq!(p.borrow().value, 10);

    list.clear(&slab);
    slab.free(h1);
    slab.free(h2);
    slab.free(p);
}

#[test]
fn unlink_middle() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h2 = list.try_push_back(&slab, 20).unwrap();
    let h3 = list.try_push_back(&slab, 30).unwrap();

    list.unlink(&h2, &slab);
    assert_eq!(list.len(), 2);
    assert_eq!(list.front().unwrap().value, 10);
    assert_eq!(list.back().unwrap().value, 30);

    // h2 is still valid (user still holds a ref)
    assert_eq!(h2.borrow().value, 20);

    list.clear(&slab);
    slab.free(h1);
    slab.free(h2);
    slab.free(h3);
}

#[test]
fn link_after() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h3 = list.try_push_back(&slab, 30).unwrap();
    let h2 = slab.alloc(ListNode::new(20));

    list.link_after(&h1, &h2);
    assert_eq!(list.len(), 3);

    let p1 = list.pop_front().unwrap();
    let p2 = list.pop_front().unwrap();
    let p3 = list.pop_front().unwrap();
    assert_eq!(p1.borrow().value, 10);
    assert_eq!(p2.borrow().value, 20);
    assert_eq!(p3.borrow().value, 30);

    slab.free(h1);
    slab.free(h2);
    slab.free(h3);
    slab.free(p1);
    slab.free(p2);
    slab.free(p3);
}

#[test]
fn link_before() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h3 = list.try_push_back(&slab, 30).unwrap();
    let h2 = slab.alloc(ListNode::new(20));

    list.link_before(&h3, &h2);
    assert_eq!(list.len(), 3);

    let p1 = list.pop_front().unwrap();
    let p2 = list.pop_front().unwrap();
    let p3 = list.pop_front().unwrap();
    assert_eq!(p1.borrow().value, 10);
    assert_eq!(p2.borrow().value, 20);
    assert_eq!(p3.borrow().value, 30);

    slab.free(h1);
    slab.free(h2);
    slab.free(h3);
    slab.free(p1);
    slab.free(p2);
    slab.free(p3);
}

#[test]
fn move_to_front() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h2 = list.try_push_back(&slab, 20).unwrap();
    let h3 = list.try_push_back(&slab, 30).unwrap();

    list.move_to_front(&h3);
    assert_eq!(list.front().unwrap().value, 30);
    assert_eq!(list.back().unwrap().value, 20);

    list.clear(&slab);
    slab.free(h1);
    slab.free(h2);
    slab.free(h3);
}

#[test]
fn move_to_back() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h2 = list.try_push_back(&slab, 20).unwrap();
    let h3 = list.try_push_back(&slab, 30).unwrap();

    list.move_to_back(&h1);
    assert_eq!(list.front().unwrap().value, 20);
    assert_eq!(list.back().unwrap().value, 10);

    list.clear(&slab);
    slab.free(h1);
    slab.free(h2);
    slab.free(h3);
}

#[test]
fn cursor_forward_traversal() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h2 = list.try_push_back(&slab, 20).unwrap();
    let h3 = list.try_push_back(&slab, 30).unwrap();

    let mut values = Vec::new();
    let mut cursor = list.cursor();
    while cursor.advance() {
        values.push(cursor.current().unwrap().value);
    }
    assert_eq!(values, vec![10, 20, 30]);

    let _ = cursor;
    list.clear(&slab);
    slab.free(h1);
    slab.free(h2);
    slab.free(h3);
}

#[test]
fn cursor_remove() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h2 = list.try_push_back(&slab, 20).unwrap();
    let h3 = list.try_push_back(&slab, 30).unwrap();

    let mut cursor = list.cursor();
    cursor.advance(); // at 10
    cursor.advance(); // at 20
    let removed = cursor.remove(); // remove 20, auto-advance to 30
    assert_eq!(removed.borrow().value, 20);
    assert_eq!(cursor.current().unwrap().value, 30);

    let _ = cursor;
    assert_eq!(list.len(), 2);

    list.clear(&slab);
    slab.free(h1);
    slab.free(h2);
    slab.free(h3);
    slab.free(removed);
}

#[test]
#[should_panic(expected = "already linked")]
fn double_link_panics() {
    let slab = make_u64_slab();
    let mut list = List::new();
    let h = slab.alloc(ListNode::new(1));
    list.link_back(&h);
    list.link_back(&h); // should panic
}

#[test]
#[should_panic(expected = "not linked to this list")]
fn unlink_wrong_list_panics() {
    let slab = make_u64_slab();
    let mut list1 = List::new();
    let mut list2 = List::new();
    let h = slab.alloc(ListNode::new(1));
    list1.link_back(&h);
    list2.unlink(&h, &slab); // should panic
}

#[test]
fn contains() {
    let slab = make_u64_slab();
    let mut list = List::new();
    let h = slab.alloc(ListNode::new(42));

    assert!(!list.contains(&h));
    list.link_back(&h);
    assert!(list.contains(&h));

    list.clear(&slab);
    slab.free(h);
}

#[test]
fn is_head_is_tail() {
    let slab = make_u64_slab();
    let mut list = List::new();

    let h1 = list.try_push_back(&slab, 10).unwrap();
    let h2 = list.try_push_back(&slab, 20).unwrap();

    assert!(list.is_head(&h1));
    assert!(!list.is_head(&h2));
    assert!(list.is_tail(&h2));
    assert!(!list.is_tail(&h1));

    list.clear(&slab);
    slab.free(h1);
    slab.free(h2);
}
