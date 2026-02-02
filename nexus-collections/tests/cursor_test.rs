//! Comprehensive tests for the List cursor API.
//!
//! Tests simulate order book workflows where:
//! - Orders have id, price, quantity
//! - Orders are added to price level queues
//! - Orders are filled (quantity reduced) via cursor iteration
//! - Fully filled orders (qty == 0) are removed during iteration
//! - An index HashMap tracks order_id -> ListSlot for O(1) cancellation

use nexus_collections::list_allocator;
use serial_test::serial;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub id: u64,
    pub price: f64,
    pub qty: u64,
}

list_allocator!(orders, Order);

// =============================================================================
// Helper functions for cleanup
// =============================================================================

/// Unlinks all remaining slots and cleans up.
fn cleanup_list(list: &mut orders::List, index: &mut HashMap<u64, orders::ListSlot>) {
    while let Some(detached) = list.pop_front() {
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        let _ = node.take();
    }
}

// =============================================================================
// Test 1: Basic forward iteration with cursor.next() and guard.read()
// =============================================================================

#[test]
#[serial]
fn test_basic_forward_iteration() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1, 2, 3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0 * i as f64,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Iterate forward, collect IDs
    let mut collected = Vec::new();
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            let id = guard.read(|o| o.id);
            collected.push(id);
        }
    }

    assert_eq!(collected, vec![1, 2, 3]);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 2: Basic backward iteration with cursor.prev()
// =============================================================================

#[test]
#[serial]
fn test_basic_backward_iteration() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1, 2, 3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0 * i as f64,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Iterate backward, collect IDs
    let mut collected = Vec::new();
    {
        let mut cursor = list.cursor_back();
        while let Some(guard) = cursor.prev() {
            let id = guard.read(|o| o.id);
            collected.push(id);
        }
    }

    assert_eq!(collected, vec![3, 2, 1]);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 3: guard.write() to modify orders in place
// =============================================================================

#[test]
#[serial]
fn test_write_modification() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Write to each order, incrementing qty
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            guard.write(|o| {
                o.qty += 5;
            });
        }
    }

    // Verify all quantities were updated
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            let qty = guard.read(|o| o.qty);
            assert_eq!(qty, 15);
        }
    }

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 4: guard.remove() unconditional removal during iteration
// =============================================================================

#[test]
#[serial]
fn test_unconditional_remove() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1, 2, 3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Remove all via cursor
    let mut removed_ids = Vec::new();
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            let detached = guard.remove();
            let node = detached.take(|o| index.remove(&o.id).unwrap());
            let order = node.take();
            removed_ids.push(order.id);
        }
    }

    assert_eq!(removed_ids, vec![1, 2, 3]);
    assert!(list.is_empty());
    assert!(index.is_empty());

    orders::shutdown().unwrap();
}

// =============================================================================
// Test 5: guard.read_remove_if() - remove orders matching a condition
// =============================================================================

#[test]
#[serial]
fn test_read_remove_if() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders with varying quantities
    for i in 1..=5 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: i * 2, // qty = 2, 4, 6, 8, 10
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Remove orders with qty <= 4 (orders 1 and 2)
    let mut removed_ids = Vec::new();
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            if let Some(detached) = guard.read_remove_if(|o| o.qty <= 4) {
                let node = detached.take(|o| index.remove(&o.id).unwrap());
                let order = node.take();
                removed_ids.push(order.id);
            }
        }
    }

    assert_eq!(removed_ids, vec![1, 2]);
    assert_eq!(list.len(), 3);

    // Verify remaining orders
    let remaining: Vec<u64> = {
        let mut ids = Vec::new();
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            ids.push(guard.read(|o| o.id));
        }
        ids
    };
    assert_eq!(remaining, vec![3, 4, 5]);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 6: guard.write_remove_if() - partial fill then remove if fully filled
// =============================================================================

#[test]
#[serial]
fn test_write_remove_if_partial_fill() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders with qty = 10
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Fill each order by 8. Orders with qty <= fill become fully filled.
    let fill_qty = 8;
    let mut fully_filled = Vec::new();

    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            if let Some(detached) = guard.write_remove_if(|o| {
                o.qty = o.qty.saturating_sub(fill_qty);
                o.qty == 0
            }) {
                let node = detached.take(|o| index.remove(&o.id).unwrap());
                let order = node.take();
                fully_filled.push(order.id);
            }
        }
    }

    // All orders had qty=10, fill=8, so qty=2 after. None removed.
    assert!(fully_filled.is_empty());
    assert_eq!(list.len(), 3);

    // Now fill remaining 5. Orders with qty <= 5 become fully filled.
    let fill_qty = 5;
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            if let Some(detached) = guard.write_remove_if(|o| {
                o.qty = o.qty.saturating_sub(fill_qty);
                o.qty == 0
            }) {
                let node = detached.take(|o| index.remove(&o.id).unwrap());
                let order = node.take();
                fully_filled.push(order.id);
            }
        }
    }

    // All had qty=2, fill=5, so all become 0 and are removed
    assert_eq!(fully_filled, vec![1, 2, 3]);
    assert!(list.is_empty());
    assert!(index.is_empty());

    orders::shutdown().unwrap();
}

// =============================================================================
// Test 7: Mixed operations - iterate forward, remove some, continue
// =============================================================================

#[test]
#[serial]
fn test_mixed_operations() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1-5
    for i in 1..=5 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: i * 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    let mut read_ids = Vec::new();
    let mut removed_ids = Vec::new();
    let mut written_ids = Vec::new();

    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            if let Some(detached) = guard.read_remove_if(|o| {
                read_ids.push(o.id);
                o.id % 2 == 0 // Remove even IDs
            }) {
                let node = detached.take(|o| index.remove(&o.id).unwrap());
                let order = node.take();
                removed_ids.push(order.id);
            }
        }
    }

    // Write to remaining (odd) IDs
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            guard.write(|o| {
                written_ids.push(o.id);
                o.qty += 100;
            });
        }
    }

    assert_eq!(read_ids, vec![1, 2, 3, 4, 5]);
    assert_eq!(removed_ids, vec![2, 4]);
    assert_eq!(written_ids, vec![1, 3, 5]);
    assert_eq!(list.len(), 3);

    // Verify modified quantities
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            let (id, qty) = guard.read(|o| (o.id, o.qty));
            // Original qty was id*10, +100 = id*10 + 100
            assert_eq!(qty, id * 10 + 100);
        }
    }

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 8: Gap state traversal - after removal, both next() and prev() work
// =============================================================================

#[test]
#[serial]
fn test_gap_state_forward_then_backward() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1, 2, 3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    let mut visited = Vec::new();
    {
        let mut cursor = list.cursor();

        // Skip to order 2 and remove it
        cursor.next().unwrap().skip(); // order 1
        let guard = cursor.next().unwrap(); // order 2
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        let _ = node.take();

        // Now in Gap state. prev() should return order 1
        if let Some(guard) = cursor.prev() {
            let id = guard.read(|o| o.id);
            visited.push(("prev_after_remove", id));
        }
    }

    // Verify: after removing order 2 and calling prev(), we get order 1
    assert_eq!(visited, vec![("prev_after_remove", 1)]);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_gap_state_backward_then_forward() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1, 2, 3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    let mut visited = Vec::new();
    {
        let mut cursor = list.cursor();

        // Move to order 2 and remove it
        cursor.next().unwrap().skip(); // order 1
        let guard = cursor.next().unwrap(); // order 2
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        let _ = node.take();

        // Now in Gap state. next() should return order 3
        if let Some(guard) = cursor.next() {
            let id = guard.read(|o| o.id);
            visited.push(("next_after_remove", id));
        }
    }

    assert_eq!(visited, vec![("next_after_remove", 3)]);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 9: Empty list edge cases
// =============================================================================

#[test]
#[serial]
fn test_empty_list_cursor() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();

    // cursor() on empty list, next() returns None
    {
        let mut cursor = list.cursor();
        assert!(cursor.next().is_none());
        // Already at AfterEnd, next() still returns None
        assert!(cursor.next().is_none());
    }

    // cursor_back() on empty list, prev() returns None
    {
        let mut cursor = list.cursor_back();
        assert!(cursor.prev().is_none());
        // Already at BeforeStart, prev() still returns None
        assert!(cursor.prev().is_none());
    }

    orders::shutdown().unwrap();
}

// =============================================================================
// Test 10: Single element list edge cases
// =============================================================================

#[test]
#[serial]
fn test_single_element_cursor() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    let node = orders::create_node(Order {
        id: 1,
        price: 100.0,
        qty: 10,
    })
    .unwrap();
    let slot = list.link_back(node);
    index.insert(1, slot);

    // Forward iteration
    {
        let mut cursor = list.cursor();
        let guard = cursor.next().unwrap();
        let id = guard.read(|o| o.id);
        assert_eq!(id, 1);
        // No more elements
        assert!(cursor.next().is_none());
        // Still at AfterEnd
        assert!(cursor.next().is_none());
    }

    // Backward iteration
    {
        let mut cursor = list.cursor_back();
        let guard = cursor.prev().unwrap();
        let id = guard.read(|o| o.id);
        assert_eq!(id, 1);
        // No more elements
        assert!(cursor.prev().is_none());
        // Still at BeforeStart
        assert!(cursor.prev().is_none());
    }

    // Forward then backward
    {
        let mut cursor = list.cursor();
        let guard = cursor.next().unwrap();
        guard.skip();
        // Now at element 1, prev() should return None (nothing before)
        assert!(cursor.prev().is_none());
    }

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_single_element_remove() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    let node = orders::create_node(Order {
        id: 1,
        price: 100.0,
        qty: 10,
    })
    .unwrap();
    let slot = list.link_back(node);
    index.insert(1, slot);

    // Remove the only element
    {
        let mut cursor = list.cursor();
        let guard = cursor.next().unwrap();
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        let _ = node.take();

        // In Gap state, both prev and next should be None (list is empty)
        assert!(cursor.prev().is_none());
        assert!(cursor.next().is_none());
    }

    assert!(list.is_empty());
    assert!(index.is_empty());

    orders::shutdown().unwrap();
}

// =============================================================================
// Test 11: Removal of head/tail during iteration
// =============================================================================

#[test]
#[serial]
fn test_remove_head_during_iteration() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1, 2, 3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    let mut remaining_ids = Vec::new();
    {
        let mut cursor = list.cursor();
        let guard = cursor.next().unwrap(); // order 1 (head)
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        let order = node.take();
        assert_eq!(order.id, 1);

        // Continue iteration - should get 2 and 3
        while let Some(guard) = cursor.next() {
            remaining_ids.push(guard.read(|o| o.id));
        }
    }

    assert_eq!(remaining_ids, vec![2, 3]);
    assert_eq!(list.len(), 2);

    // Verify new head is order 2
    let new_head = list.front(|o| o.id);
    assert_eq!(new_head, Some(2));

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_remove_tail_during_iteration() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1, 2, 3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Remove tail via backward iteration
    {
        let mut cursor = list.cursor_back();
        let guard = cursor.prev().unwrap(); // order 3 (tail)
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        let order = node.take();
        assert_eq!(order.id, 3);
    }

    assert_eq!(list.len(), 2);

    // Verify new tail is order 2
    let new_tail = list.back(|o| o.id);
    assert_eq!(new_tail, Some(2));

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_remove_middle_during_iteration() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1, 2, 3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Remove middle (order 2)
    {
        let mut cursor = list.cursor();
        cursor.next().unwrap().skip(); // skip order 1
        let guard = cursor.next().unwrap(); // order 2
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        let order = node.take();
        assert_eq!(order.id, 2);
    }

    assert_eq!(list.len(), 2);

    // Verify order is now 1, 3
    let mut ids = Vec::new();
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            ids.push(guard.read(|o| o.id));
        }
    }
    assert_eq!(ids, vec![1, 3]);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 12: Index cleanup - verify HashMap stays in sync after removals
// =============================================================================

#[test]
#[serial]
fn test_index_sync_after_removals() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index: HashMap<u64, orders::ListSlot> = HashMap::new();

    // Add orders 1-10
    for i in 1..=10 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0 * i as f64,
            qty: i * 5,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    assert_eq!(list.len(), 10);
    assert_eq!(index.len(), 10);

    // Remove orders 2, 4, 6, 8, 10 via cursor
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            if let Some(detached) = guard.read_remove_if(|o| o.id % 2 == 0) {
                let node = detached.take(|o| index.remove(&o.id).unwrap());
                let _ = node.take();
            }
        }
    }

    // Verify counts match
    assert_eq!(list.len(), 5);
    assert_eq!(index.len(), 5);

    // Verify only odd IDs remain in index
    for i in 1..=10 {
        if i % 2 == 1 {
            assert!(index.contains_key(&i), "Expected index to contain {}", i);
        } else {
            assert!(
                !index.contains_key(&i),
                "Expected index NOT to contain {}",
                i
            );
        }
    }

    // Verify list contains only odd IDs in order
    let mut ids = Vec::new();
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            ids.push(guard.read(|o| o.id));
        }
    }
    assert_eq!(ids, vec![1, 3, 5, 7, 9]);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 13: Order book workflow - realistic fill scenario
// =============================================================================

#[test]
#[serial]
fn test_order_book_fill_workflow() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index: HashMap<u64, orders::ListSlot> = HashMap::new();

    // Create a price level with multiple orders
    // Orders are in time priority (FIFO)
    let orders_data = vec![
        Order {
            id: 100,
            price: 50.0,
            qty: 100,
        },
        Order {
            id: 101,
            price: 50.0,
            qty: 50,
        },
        Order {
            id: 102,
            price: 50.0,
            qty: 200,
        },
        Order {
            id: 103,
            price: 50.0,
            qty: 75,
        },
    ];

    for order in orders_data {
        let id = order.id;
        let node = orders::create_node(order).unwrap();
        let slot = list.link_back(node);
        index.insert(id, slot);
    }

    // Incoming fill for 175 shares
    // Should fill: order 100 (100), order 101 (50), order 102 (25 partial)
    let mut fill_remaining = 175u64;
    let mut filled_orders = Vec::new();

    {
        let mut cursor = list.cursor();
        while fill_remaining > 0 {
            let Some(guard) = cursor.next() else { break };

            if let Some(detached) = guard.write_remove_if(|order| {
                if order.qty <= fill_remaining {
                    // Fully filled
                    fill_remaining -= order.qty;
                    order.qty = 0;
                    true // Remove
                } else {
                    // Partially filled
                    order.qty -= fill_remaining;
                    fill_remaining = 0;
                    false // Keep
                }
            }) {
                let node = detached.take(|o| index.remove(&o.id).unwrap());
                let order = node.take();
                filled_orders.push((order.id, "full"));
            }
        }
    }

    // Verify fill results
    assert_eq!(fill_remaining, 0);
    assert_eq!(filled_orders, vec![(100, "full"), (101, "full")]);

    // Verify remaining orders
    assert_eq!(list.len(), 2);
    assert_eq!(index.len(), 2);

    // Order 102 should have 175 remaining (200 - 25)
    {
        let slot = index.get(&102).unwrap();
        let qty = list.read(slot, |o| o.qty);
        assert_eq!(qty, 175); // 200 - 25
    }

    // Order 103 should be unchanged
    {
        let slot = index.get(&103).unwrap();
        let qty = list.read(slot, |o| o.qty);
        assert_eq!(qty, 75);
    }

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 14: Bidirectional traversal
// =============================================================================

#[test]
#[serial]
fn test_bidirectional_traversal() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1-5
    for i in 1..=5 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Traverse forward to 3, then backward to 1, then forward to 5
    let mut path = Vec::new();
    {
        let mut cursor = list.cursor();

        // Forward to 3
        for _ in 0..3 {
            if let Some(guard) = cursor.next() {
                path.push(("fwd", guard.read(|o| o.id)));
            }
        }

        // Backward to 1 (currently at 3, prev gives 2, then 1)
        for _ in 0..2 {
            if let Some(guard) = cursor.prev() {
                path.push(("back", guard.read(|o| o.id)));
            }
        }

        // Forward to 5 (currently at 1, next gives 2, 3, 4, 5)
        for _ in 0..4 {
            if let Some(guard) = cursor.next() {
                path.push(("fwd", guard.read(|o| o.id)));
            }
        }
    }

    assert_eq!(
        path,
        vec![
            ("fwd", 1),
            ("fwd", 2),
            ("fwd", 3),
            ("back", 2),
            ("back", 1),
            ("fwd", 2),
            ("fwd", 3),
            ("fwd", 4),
            ("fwd", 5),
        ]
    );

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 15: Skip operation
// =============================================================================

#[test]
#[serial]
fn test_skip_operation() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1-3
    for i in 1..=3 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Skip first two, read third
    let mut result = None;
    {
        let mut cursor = list.cursor();
        cursor.next().unwrap().skip();
        cursor.next().unwrap().skip();
        if let Some(guard) = cursor.next() {
            result = Some(guard.read(|o| o.id));
        }
    }

    assert_eq!(result, Some(3));

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 16: Multiple consecutive removals
// =============================================================================

#[test]
#[serial]
fn test_multiple_consecutive_removals() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1-5
    for i in 1..=5 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Remove 2, 3, 4 consecutively
    let mut removed = Vec::new();
    {
        let mut cursor = list.cursor();

        // Skip to 2
        cursor.next().unwrap().skip(); // 1

        // Remove 2
        let guard = cursor.next().unwrap();
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        removed.push(node.take().id);

        // Remove 3 (next from gap)
        let guard = cursor.next().unwrap();
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        removed.push(node.take().id);

        // Remove 4 (next from gap)
        let guard = cursor.next().unwrap();
        let detached = guard.remove();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        removed.push(node.take().id);

        // Next should be 5
        let guard = cursor.next().unwrap();
        let id = guard.read(|o| o.id);
        assert_eq!(id, 5);
    }

    assert_eq!(removed, vec![2, 3, 4]);
    assert_eq!(list.len(), 2);

    // Verify remaining
    let mut ids = Vec::new();
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            ids.push(guard.read(|o| o.id));
        }
    }
    assert_eq!(ids, vec![1, 5]);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 17: Remove all elements via cursor
// =============================================================================

#[test]
#[serial]
fn test_remove_all_via_cursor() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    // Add orders 1-5
    for i in 1..=5 {
        let node = orders::create_node(Order {
            id: i,
            price: 100.0,
            qty: 10,
        })
        .unwrap();
        let slot = list.link_back(node);
        index.insert(i, slot);
    }

    // Remove all
    let mut removed_count = 0;
    {
        let mut cursor = list.cursor();
        while let Some(guard) = cursor.next() {
            let detached = guard.remove();
            let node = detached.take(|o| index.remove(&o.id).unwrap());
            let _ = node.take();
            removed_count += 1;
        }
    }

    assert_eq!(removed_count, 5);
    assert!(list.is_empty());
    assert!(index.is_empty());

    // Cursor on empty list should work
    {
        let mut cursor = list.cursor();
        assert!(cursor.next().is_none());
    }

    orders::shutdown().unwrap();
}

// =============================================================================
// Test 18: write_remove_if keeps element when predicate returns false
// =============================================================================

#[test]
#[serial]
fn test_write_remove_if_keeps_element() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    let node = orders::create_node(Order {
        id: 1,
        price: 100.0,
        qty: 100,
    })
    .unwrap();
    let slot = list.link_back(node);
    index.insert(1, slot);

    // Modify but don't remove
    {
        let mut cursor = list.cursor();
        let guard = cursor.next().unwrap();
        let result = guard.write_remove_if(|o| {
            o.qty -= 50;
            false // Don't remove
        });
        assert!(result.is_none());

        // Guard consumed, next() will move forward
        assert!(cursor.next().is_none()); // No more elements
    }

    // Verify modification persisted
    let qty = list.front(|o| o.qty);
    assert_eq!(qty, Some(50));

    assert_eq!(list.len(), 1);
    assert_eq!(index.len(), 1);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 19: read_remove_if keeps element when predicate returns false
// =============================================================================

#[test]
#[serial]
fn test_read_remove_if_keeps_element() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    let node = orders::create_node(Order {
        id: 1,
        price: 100.0,
        qty: 100,
    })
    .unwrap();
    let slot = list.link_back(node);
    index.insert(1, slot);

    // Check but don't remove
    let mut checked_id = None;
    {
        let mut cursor = list.cursor();
        let guard = cursor.next().unwrap();
        let result = guard.read_remove_if(|o| {
            checked_id = Some(o.id);
            false // Don't remove
        });
        assert!(result.is_none());
    }

    assert_eq!(checked_id, Some(1));
    assert_eq!(list.len(), 1);
    assert_eq!(index.len(), 1);

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

// =============================================================================
// Test 20: Cursor boundary conditions
// =============================================================================

#[test]
#[serial]
fn test_cursor_boundary_prev_at_before_start() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    let node = orders::create_node(Order {
        id: 1,
        price: 100.0,
        qty: 10,
    })
    .unwrap();
    let slot = list.link_back(node);
    index.insert(1, slot);

    // cursor() starts BeforeStart, prev() should return None
    {
        let mut cursor = list.cursor();
        assert!(cursor.prev().is_none());
        // Still at BeforeStart, next() should work
        assert!(cursor.next().is_some());
    }

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}

#[test]
#[serial]
fn test_cursor_boundary_next_at_after_end() {
    orders::init().bounded(100).build();

    let mut list = orders::List::new();
    let mut index = HashMap::new();

    let node = orders::create_node(Order {
        id: 1,
        price: 100.0,
        qty: 10,
    })
    .unwrap();
    let slot = list.link_back(node);
    index.insert(1, slot);

    // cursor_back() starts AfterEnd, next() should return None
    {
        let mut cursor = list.cursor_back();
        assert!(cursor.next().is_none());
        // Still at AfterEnd, prev() should work
        assert!(cursor.prev().is_some());
    }

    cleanup_list(&mut list, &mut index);
    orders::shutdown().unwrap();
}
