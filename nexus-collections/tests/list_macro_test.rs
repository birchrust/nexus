//! Tests for the create_list! macro and List functionality.

use nexus_collections::create_list;

#[derive(Debug, PartialEq)]
pub struct Order {
    pub id: u64,
    pub price: f64,
}

create_list!(orders, Order);

#[test]
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
