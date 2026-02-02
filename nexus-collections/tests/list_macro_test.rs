//! Tests for the list_allocator! macro and List functionality.

#![allow(clippy::float_cmp)] // Exact f64 constant comparisons in tests are fine

use nexus_collections::list_allocator;
use serial_test::serial;

#[derive(Debug, PartialEq)]
pub struct Order {
    pub id: u64,
    pub price: f64,
}

list_allocator!(orders, Order);

#[test]
#[serial]
fn test_basic_list_operations() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);

    // Create and link a node
    let node = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let slot1 = list.link_back(node);

    assert!(!list.is_empty());
    assert_eq!(list.len(), 1);

    // Read via closure
    let price = list.read(&slot1, |o| o.price);
    assert_eq!(price, 100.0);

    // Add another node
    let node2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();
    let slot2 = list.link_back(node2);
    assert_eq!(list.len(), 2);

    // Check front/back
    let front_id = list.front(|o| o.id);
    assert_eq!(front_id, Some(1));

    let back_id = list.back(|o| o.id);
    assert_eq!(back_id, Some(2));

    // Unlink and take
    let detached = list.unlink(slot1);
    assert_eq!(list.len(), 1);

    let order = detached.take();
    assert_eq!(order.id, 1);

    // Unlink remaining
    let detached2 = list.unlink(slot2);
    let order2 = detached2.take();
    assert_eq!(order2.id, 2);

    assert!(list.is_empty());

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_link_front() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    let s2 = list.link_front(n2); // 2 should be at front now

    let front_id = list.front(|o| o.id);
    assert_eq!(front_id, Some(2));

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_write_via_closure() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let node = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let mut slot = list.link_back(node);

    // Modify via write
    list.write(&mut slot, |o| {
        o.price = 150.0;
    });

    let new_price = list.read(&slot, |o| o.price);
    assert_eq!(new_price, 150.0);

    // Cleanup
    let detached = list.unlink(slot);
    let _ = detached.take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_move_to_front() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();
    let n3 = orders::create_node(Order {
        id: 3,
        price: 300.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    let s2 = list.link_back(n2);
    let s3 = list.link_back(n3);

    // Order: 1, 2, 3
    assert_eq!(list.front(|o| o.id), Some(1));

    // Move 3 to front
    list.move_to_front(&s3);

    // Order: 3, 1, 2
    assert_eq!(list.front(|o| o.id), Some(3));

    // Move 1 to front (already at position 2)
    list.move_to_front(&s1);

    // Order: 1, 3, 2
    assert_eq!(list.front(|o| o.id), Some(1));

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();
    let _ = list.unlink(s3).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_pop_front() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = std::collections::HashMap::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    let s2 = list.link_back(n2);

    index.insert(1u64, s1);
    index.insert(2u64, s2);

    // Pop front
    let detached = list.pop_front().unwrap();
    let node = detached.take(|o| index.remove(&o.id).unwrap());
    let order = node.take();

    assert_eq!(order.id, 1);
    assert_eq!(list.len(), 1);

    // Cleanup remaining
    let detached2 = list.pop_front().unwrap();
    let node2 = detached2.take(|o| index.remove(&o.id).unwrap());
    let _ = node2.take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_pop_back() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = std::collections::HashMap::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();
    let n3 = orders::create_node(Order {
        id: 3,
        price: 300.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    let s2 = list.link_back(n2);
    let s3 = list.link_back(n3);

    index.insert(1u64, s1);
    index.insert(2u64, s2);
    index.insert(3u64, s3);

    // Pop back (should be order 3)
    let detached = list.pop_back().unwrap();
    let node = detached.take(|o| index.remove(&o.id).unwrap());
    let order = node.take();

    assert_eq!(order.id, 3);
    assert_eq!(list.len(), 2);
    assert_eq!(list.back(|o| o.id), Some(2));

    // Pop back again (should be order 2)
    let detached = list.pop_back().unwrap();
    let node = detached.take(|o| index.remove(&o.id).unwrap());
    let order = node.take();

    assert_eq!(order.id, 2);
    assert_eq!(list.len(), 1);
    assert_eq!(list.back(|o| o.id), Some(1));

    // Pop back last element
    let detached = list.pop_back().unwrap();
    let node = detached.take(|o| index.remove(&o.id).unwrap());
    let order = node.take();

    assert_eq!(order.id, 1);
    assert!(list.is_empty());

    // Pop back on empty list returns None
    assert!(list.pop_back().is_none());

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_front_mut_back_mut() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    // front_mut/back_mut on empty list returns None
    assert!(list.front_mut(|_o: &mut Order| ()).is_none());
    assert!(list.back_mut(|_o: &mut Order| ()).is_none());

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    let s2 = list.link_back(n2);

    // Modify front via front_mut
    let old_price = list.front_mut(|o| {
        let old = o.price;
        o.price = 150.0;
        old
    });
    assert_eq!(old_price, Some(100.0));
    assert_eq!(list.front(|o| o.price), Some(150.0));

    // Modify back via back_mut
    let old_price = list.back_mut(|o| {
        let old = o.price;
        o.price = 250.0;
        old
    });
    assert_eq!(old_price, Some(200.0));
    assert_eq!(list.back(|o| o.price), Some(250.0));

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_link_after() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();
    let n3 = orders::create_node(Order {
        id: 3,
        price: 300.0,
    })
    .unwrap();

    // Start with just order 1
    let s1 = list.link_back(n1);

    // Link order 2 after order 1
    let s2 = list.link_after(&s1, n2);

    // Order should be: 1, 2
    assert_eq!(list.front(|o| o.id), Some(1));
    assert_eq!(list.back(|o| o.id), Some(2));
    assert_eq!(list.len(), 2);

    // Link order 3 after order 1 (between 1 and 2)
    let s3 = list.link_after(&s1, n3);

    // Order should be: 1, 3, 2
    assert_eq!(list.len(), 3);

    // Verify order by iterating
    let mut ids = Vec::new();
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            ids.push(guard.read(|o| o.id));
        }
    }
    assert_eq!(ids, vec![1, 3, 2]);

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();
    let _ = list.unlink(s3).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_link_before() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();
    let n3 = orders::create_node(Order {
        id: 3,
        price: 300.0,
    })
    .unwrap();

    // Start with just order 1
    let s1 = list.link_back(n1);

    // Link order 2 before order 1
    let s2 = list.link_before(&s1, n2);

    // Order should be: 2, 1
    assert_eq!(list.front(|o| o.id), Some(2));
    assert_eq!(list.back(|o| o.id), Some(1));
    assert_eq!(list.len(), 2);

    // Link order 3 before order 1 (between 2 and 1)
    let s3 = list.link_before(&s1, n3);

    // Order should be: 2, 3, 1
    assert_eq!(list.len(), 3);

    // Verify order by iterating
    let mut ids = Vec::new();
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            ids.push(guard.read(|o| o.id));
        }
    }
    assert_eq!(ids, vec![2, 3, 1]);

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();
    let _ = list.unlink(s3).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_move_to_back() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();
    let n3 = orders::create_node(Order {
        id: 3,
        price: 300.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    let s2 = list.link_back(n2);
    let s3 = list.link_back(n3);

    // Order: 1, 2, 3
    assert_eq!(list.front(|o| o.id), Some(1));
    assert_eq!(list.back(|o| o.id), Some(3));

    // Move 1 to back
    list.move_to_back(&s1);

    // Order: 2, 3, 1
    assert_eq!(list.front(|o| o.id), Some(2));
    assert_eq!(list.back(|o| o.id), Some(1));

    // Move 3 to back (from middle)
    list.move_to_back(&s3);

    // Order: 2, 1, 3
    assert_eq!(list.front(|o| o.id), Some(2));
    assert_eq!(list.back(|o| o.id), Some(3));

    // Move 3 to back (already at back - should be no-op)
    list.move_to_back(&s3);

    // Order still: 2, 1, 3
    assert_eq!(list.front(|o| o.id), Some(2));
    assert_eq!(list.back(|o| o.id), Some(3));

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();
    let _ = list.unlink(s3).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_is_head_is_tail() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();
    let n3 = orders::create_node(Order {
        id: 3,
        price: 300.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);

    // Single element is both head and tail
    assert!(list.is_head(&s1));
    assert!(list.is_tail(&s1));

    let s2 = list.link_back(n2);

    // s1 is head, s2 is tail
    assert!(list.is_head(&s1));
    assert!(!list.is_tail(&s1));
    assert!(!list.is_head(&s2));
    assert!(list.is_tail(&s2));

    let s3 = list.link_back(n3);

    // s1 is head, s3 is tail, s2 is middle
    assert!(list.is_head(&s1));
    assert!(!list.is_tail(&s1));
    assert!(!list.is_head(&s2));
    assert!(!list.is_tail(&s2));
    assert!(!list.is_head(&s3));
    assert!(list.is_tail(&s3));

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();
    let _ = list.unlink(s3).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_detached_try_take_success() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = std::collections::HashMap::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    index.insert(1u64, s1);

    // Pop and use try_take with successful lookup
    let detached = list.pop_front().unwrap();
    let result = detached.try_take(|o| index.remove(&o.id));

    assert!(result.is_some());
    let node = result.unwrap();
    let order = node.take();
    assert_eq!(order.id, 1);

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_detached_try_take_failure() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index: std::collections::HashMap<u64, orders::ListSlot> =
        std::collections::HashMap::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    index.insert(1u64, s1);

    // Pop and use try_take with failing lookup (wrong key)
    let detached = list.pop_front().unwrap();
    let result = detached.try_take(|o| index.remove(&(o.id + 999))); // Wrong key

    // try_take returns None when lookup fails
    assert!(result.is_none());

    // Note: This leaks the node in the slab (documented behavior).
    // In real code, this indicates a bug in index tracking.

    // Clean up the index (slot is still there but orphaned in slab)
    index.clear();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_list_ids_are_unique() {
    orders::init().bounded(100).build();

    let list1 = orders::List::new();
    let list2 = orders::List::new();
    let list3 = orders::List::new();

    // Each list should have a unique ID
    assert_ne!(list1.id(), list2.id());
    assert_ne!(list2.id(), list3.id());
    assert_ne!(list1.id(), list3.id());

    // IDs should not be NONE
    assert!(!list1.id().is_none());
    assert!(!list2.id().is_none());
    assert!(!list3.id().is_none());

    assert!(list1.id().is_some());
    assert!(list2.id().is_some());
    assert!(list3.id().is_some());

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_link_after_at_tail() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    // s1 is now tail

    // link_after at tail should make n2 the new tail
    let s2 = list.link_after(&s1, n2);

    assert_eq!(list.back(|o| o.id), Some(2));
    assert!(list.is_tail(&s2));
    assert!(!list.is_tail(&s1));

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_link_before_at_head() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    // s1 is now head

    // link_before at head should make n2 the new head
    let s2 = list.link_before(&s1, n2);

    assert_eq!(list.front(|o| o.id), Some(2));
    assert!(list.is_head(&s2));
    assert!(!list.is_head(&s1));

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_move_to_front_already_at_front() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();
    let n2 = orders::create_node(Order {
        id: 2,
        price: 200.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);
    let s2 = list.link_back(n2);

    // s1 is at front
    assert!(list.is_head(&s1));

    // move_to_front when already at front should be no-op
    list.move_to_front(&s1);

    // Still in same order
    assert!(list.is_head(&s1));
    assert!(list.is_tail(&s2));
    assert_eq!(list.front(|o| o.id), Some(1));
    assert_eq!(list.back(|o| o.id), Some(2));

    // Cleanup
    let _ = list.unlink(s1).take();
    let _ = list.unlink(s2).take();

    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_single_element_move_operations() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    let n1 = orders::create_node(Order {
        id: 1,
        price: 100.0,
    })
    .unwrap();

    let s1 = list.link_back(n1);

    // Single element - move_to_front should be no-op
    list.move_to_front(&s1);
    assert!(list.is_head(&s1));
    assert!(list.is_tail(&s1));

    // Single element - move_to_back should be no-op
    list.move_to_back(&s1);
    assert!(list.is_head(&s1));
    assert!(list.is_tail(&s1));

    // Cleanup
    let _ = list.unlink(s1).take();

    orders::shutdown().unwrap();
}
