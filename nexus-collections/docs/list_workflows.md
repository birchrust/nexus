# List Workflow Examples

Detailed examples showing the closure-based access pattern and how the borrow
checker prevents aliasing violations.

---

## ⚠️ USER INVARIANTS - READ THIS FIRST

You must uphold these invariants for correct behavior:

| # | Invariant | If Violated (Safe API) | If Violated (Unchecked API) |
|---|-----------|------------------------|----------------------------|
| 1 | **Consume all guards** (`Detached`, `CursorGuard`) | Orphaned data (leak) | Orphaned data (leak) |
| 2 | **Unlink slots before dropping** | Panic: "slot is invalid" | **UB** |
| 3 | **Return correct slot from `take()`** | Corruption | Corruption |
| 4 | **Keep your index in sync** | Panic or wrong data | **UB** or wrong data |

### Safe vs Unchecked Access

**Safe API (default):** Uses slab's checked access. If you violate invariants
(e.g., drop a linked slot), you get a panic with a clear message instead of UB.
Small runtime cost (one validity check per access).

```rust
list.read(&slot, |order| order.price);      // Panics if slot is stale
list.write(&mut slot, |order| { ... });     // Panics if slot is stale
```

**Unchecked API (opt-in):** Zero overhead, but you take full responsibility.
Violating invariants causes UB.

```rust
unsafe { list.read_unchecked(&slot, |order| order.price) };  // UB if stale
```

**Why no panic-on-drop?** To avoid process abort if your code panics while
holding a guard (double-panic = abort). We catch violations at access time
instead.

---

## Setup

```rust
use nexus_collections::{List, DetachedListNode, ListSlot, Node};
use nexus_slab::Slab;
use std::collections::HashMap;

#[derive(Debug)]
struct Order {
    id: u64,
    price: f64,
    qty: u32,
}

// Create slab and list
let slab: Slab<Node<Order>> = Slab::with_capacity(10_000);
let mut list: List<Order, _> = List::new(&slab);

// User's external index: OrderId -> ListSlot
let mut index: HashMap<u64, ListSlot<Order>> = HashMap::new();
```

---

## 1. Insert and Track

```rust
// Create order
let order = Order { id: 1001, price: 100.0, qty: 50 };
let order_id = order.id;

// Insert into slab (detached - not in any list yet)
let detached: DetachedListNode<Order> = DetachedListNode::new(&slab, order)?;

// Link to list (type-state transition: Detached -> Linked)
let slot: ListSlot<Order> = list.link_back(detached);

// Store in user's index
index.insert(order_id, slot);
```

**Type-state enforced:** After `link_back`, `detached` is consumed. You cannot
accidentally use it again.

```rust
// COMPILE ERROR: use of moved value
let slot2 = list.link_back(detached);  // ERROR: detached already moved
```

---

## 2. Simple Read Access

### Read via slot (from index lookup)

```rust
let slot: &ListSlot<Order> = index.get(&1001).unwrap();

// Read through the list - closure receives &Order
let price = list.read(slot, |order| order.price);
println!("Price: {}", price);

// Multiple reads are fine - &self allows concurrent borrows
let qty = list.read(slot, |order| order.qty);
let id = list.read(slot, |order| order.id);
```

### Read front/back

```rust
// front() takes closure directly, returns Option<R>
let front_id: Option<u64> = list.front(|order| order.id);

// Can also extract multiple fields
let front_info: Option<(u64, f64)> = list.front(|order| (order.id, order.price));
```

### BORROW CHECKER: Multiple reads OK

```rust
// This is fine - read() takes &self, multiple &self borrows allowed
let slot1 = index.get(&1001).unwrap();
let slot2 = index.get(&1002).unwrap();

// Both reads can happen (though sequentially in this syntax)
let p1 = list.read(slot1, |o| o.price);
let p2 = list.read(slot2, |o| o.price);
```

---

## 3. Simple Write Access

### Write via slot (from index lookup)

```rust
// Need &mut slot for write - get_mut from index
let slot: &mut ListSlot<Order> = index.get_mut(&1001).unwrap();

// Write through the list - closure receives &mut Order
list.write(slot, |order| {
    order.qty -= 10;
    order.price = 99.50;
});
```

### Write front/back

```rust
// front_mut() takes closure, returns Option<R>
list.front_mut(|order| {
    order.qty = 0;
});
```

### BORROW CHECKER: Write blocks all other access

```rust
let slot: &mut ListSlot<Order> = index.get_mut(&1001).unwrap();

list.write(slot, |order| {
    // Inside this closure:
    // - list is &mut borrowed (by write)
    // - slot is &mut borrowed (by write)
    // - order is &mut borrowed (by closure)

    // COMPILE ERROR: cannot borrow list again
    // list.read(slot, |o| o.price);  // ERROR: list already borrowed

    // COMPILE ERROR: cannot use slot again
    // let _ = slot;  // ERROR: slot already borrowed

    // COMPILE ERROR: cannot call any other list method
    // list.front(|o| o.id);  // ERROR: list already borrowed

    order.qty -= 1;
});
// After closure: all borrows released
```

### BORROW CHECKER: Can't write two slots simultaneously

```rust
let slot1: &mut ListSlot<Order> = index.get_mut(&1001).unwrap();

list.write(slot1, |order1| {
    // COMPILE ERROR: can't get another mutable borrow of index
    // let slot2 = index.get_mut(&1002).unwrap();  // ERROR

    // COMPILE ERROR: can't call write again (list is &mut borrowed)
    // list.write(???, |order2| { ... });  // ERROR

    order1.qty -= 1;
});
```

---

## 4. Read-then-Write Pattern

```rust
// Read first
let slot: &ListSlot<Order> = index.get(&1001).unwrap();
let current_qty = list.read(slot, |order| order.qty);

// Now write (need &mut slot)
let slot: &mut ListSlot<Order> = index.get_mut(&1001).unwrap();
list.write(slot, |order| {
    order.qty = current_qty - 10;
});
```

### BORROW CHECKER: Can't hold read result across write

```rust
// This pattern is PREVENTED:
let slot = index.get(&1001).unwrap();

// If read() returned &T (it doesn't!), this would be unsafe:
// let order_ref: &Order = list.read_WRONG(slot);  // Hypothetical bad API
// list.write(slot, |o| o.qty -= 1);  // Would mutate while order_ref exists
// println!("{}", order_ref.qty);  // UB: reading through invalidated ref

// Our API makes this impossible - closure consumes the reference:
let qty = list.read(slot, |order| order.qty);  // qty is u32, not &u32
// No reference escapes, write is safe
```

---

## 5. Pop Operations (Detached Guard)

### Pop front - infallible (you know it's in index)

```rust
// pop_front() unlinks the node and returns Detached guard
if let Some(detached) = list.pop_front() {
    // detached holds &Order to identify the node
    // Must call take() or try_take() - panics on drop!

    let node: DetachedListNode<Order> = detached.take(|order| {
        // Closure receives &Order to identify which slot
        // Return the ListSlot from your index
        index.remove(&order.id).unwrap()  // Panics if not found
    });

    // node is now DetachedListNode - can take() to extract data
    let order: Order = node.take();
    println!("Processed order {}: {} @ {}", order.id, order.qty, order.price);
}
```

### Pop front - fallible (handle missing from index gracefully)

```rust
if let Some(detached) = list.pop_front() {
    // try_take returns Option<DetachedListNode>
    let maybe_node: Option<DetachedListNode<Order>> = detached.try_take(|order| {
        index.remove(&order.id)  // Returns Option<ListSlot>
    });

    match maybe_node {
        Some(node) => {
            let order = node.take();
            println!("Processed: {}", order.id);
        }
        None => {
            // Index was out of sync - log error
            eprintln!("Warning: popped order not in index");
        }
    }
}
```

### BORROW CHECKER: Detached prevents use-after-pop

```rust
if let Some(detached) = list.pop_front() {
    // The node is already unlinked from the list
    // detached holds a reference to the data

    // COMPILE ERROR: can't access list while detached exists
    // list.front(|o| o.id);  // ERROR: list mutably borrowed by pop_front

    // Actually wait - pop_front returns Detached<'_, T> which borrows list
    // So we can't use list until we consume detached

    let node = detached.take(|order| index.remove(&order.id).unwrap());

    // Now list is usable again
    list.front(|o| println!("New front: {}", o.id));
}
```

### USER INVARIANT: Must consume Detached

```rust
if let Some(detached) = list.pop_front() {
    // BUG: Forgot to consume detached!
}
// detached drops without take()/try_take()
// Result: popped node is orphaned in slab (memory leak, logical corruption)
// NO PANIC - but your data structure is now corrupted

// CORRECT: Always consume
if let Some(detached) = list.pop_front() {
    let node = detached.take(|order| index.remove(&order.id).unwrap());
    process(node.take());
}
```

---

## 6. Unlink and Re-link (Move within list)

### Move from anywhere to back

```rust
// Remove slot from index (we'll re-insert with same key)
let slot: ListSlot<Order> = index.remove(&1001).unwrap();

// Unlink from current position (returns DetachedListNode)
let detached: DetachedListNode<Order> = list.unlink(slot);

// Link to back (returns new ListSlot)
let new_slot: ListSlot<Order> = list.link_back(detached);

// Update index with new slot
index.insert(1001, new_slot);
```

### Move from anywhere to front

```rust
let slot = index.remove(&1001).unwrap();
let detached = list.unlink(slot);
let new_slot = list.link_front(detached);
index.insert(1001, new_slot);
```

### BORROW CHECKER: Type-state prevents double-unlink

```rust
let slot: ListSlot<Order> = index.remove(&1001).unwrap();
let detached = list.unlink(slot);

// COMPILE ERROR: slot is moved
// let detached2 = list.unlink(slot);  // ERROR: use of moved value

// COMPILE ERROR: can't link a ListSlot (wrong type)
// list.link_back(slot);  // ERROR: expected DetachedListNode, found ListSlot
```

### BORROW CHECKER: Type-state prevents double-link

```rust
let detached = DetachedListNode::new(&slab, order)?;
let slot = list.link_back(detached);

// COMPILE ERROR: detached is moved
// let slot2 = list.link_front(detached);  // ERROR: use of moved value
```

---

## 7. Move Between Lists

```rust
let slab: Slab<Node<Order>> = Slab::with_capacity(10_000);
let mut active_orders: List<Order, _> = List::new(&slab);
let mut completed_orders: List<Order, _> = List::new(&slab);
let mut index: HashMap<u64, ListSlot<Order>> = HashMap::new();

// ... insert orders into active_orders ...

// Move order from active to completed
fn complete_order(
    order_id: u64,
    active: &mut List<Order, _>,
    completed: &mut List<Order, _>,
    index: &mut HashMap<u64, ListSlot<Order>>,
) {
    // Remove from index
    let slot = index.remove(&order_id).unwrap();

    // Unlink from active list
    let detached = active.unlink(slot);

    // Link to completed list
    let new_slot = completed.link_back(detached);

    // Re-insert into index (same order_id, new slot)
    index.insert(order_id, new_slot);
}
```

### OWNER VALIDATION: Prevents wrong-list unlink

```rust
let slot_from_list1 = index.get(&1001).unwrap();  // This slot is in list1

// At runtime (debug_assert):
list2.unlink(slot_from_list1);
// debug_assert fails: "slot belongs to different list"

// The slot has owner: ListId, list2 checks slot.owner == self.id
```

---

## 8. Cursor Traversal

### Forward iteration with read

```rust
let mut cursor = list.cursor();  // Positioned before first element

while let Some(guard) = cursor.next() {
    // guard is CursorGuard - must be consumed!
    guard.read(|order| {
        println!("Order {}: {} @ {}", order.id, order.qty, order.price);
    });
    // guard consumed by read(), cursor advanced
}
```

### Reverse iteration

```rust
let mut cursor = list.cursor_back();  // Positioned after last element

while let Some(guard) = cursor.prev() {
    guard.read(|order| {
        println!("Order {}: {} @ {}", order.id, order.qty, order.price);
    });
}
```

### BORROW CHECKER: Must consume guard before next iteration

```rust
let mut cursor = list.cursor();

while let Some(guard) = cursor.next() {
    // COMPILE ERROR if we don't consume guard:
    // The next iteration calls cursor.next() which needs &mut cursor
    // But guard borrows cursor

    guard.read(|o| println!("{}", o.id));  // Consumes guard, releases borrow
}
```

### USER INVARIANT: Must consume CursorGuard

```rust
let mut cursor = list.cursor();
if let Some(guard) = cursor.next() {
    // BUG: Forgot to call read/write/skip/remove
}
// guard drops without being consumed
// Result: cursor state may be inconsistent
// NO PANIC - but cursor behavior is now undefined

// CORRECT: Always consume
if let Some(guard) = cursor.next() {
    guard.read(|o| println!("{}", o.id));
}
```

---

## 9. Cursor Modification

### Write during traversal

```rust
let mut cursor = list.cursor();

while let Some(guard) = cursor.next() {
    guard.write(|order| {
        order.price *= 1.05;  // 5% price increase
    });
}
```

### Skip without accessing

```rust
let mut cursor = list.cursor();
let mut count = 0;

while let Some(guard) = cursor.next() {
    count += 1;
    if count <= 5 {
        guard.skip();  // Skip first 5, don't access
        continue;
    }
    guard.read(|order| {
        println!("Order after skip: {}", order.id);
    });
}
```

---

## 10. Cursor Removal

### Unconditional removal

```rust
let mut cursor = list.cursor();

while let Some(guard) = cursor.next() {
    if should_remove_order() {
        // remove() returns Detached guard
        let detached = guard.remove();

        // Must complete the transition
        if let Some(node) = detached.try_take(|order| index.remove(&order.id)) {
            let order = node.take();
            process_removed_order(order);
        }
        // Cursor is now in Gap state, next() will go to correct element
    } else {
        guard.skip();
    }
}
```

### Conditional removal with read_remove_if

```rust
let mut cursor = list.cursor();

while let Some(guard) = cursor.next() {
    // Read and conditionally remove in one operation
    if let Some(detached) = guard.read_remove_if(|order| {
        order.qty == 0  // Remove if fully filled
    }) {
        let node = detached.take(|order| index.remove(&order.id).unwrap());
        let order = node.take();
        send_fill_report(order);
    }
    // If not removed, cursor stays at current element
    // If removed, cursor is in Gap state
}
```

### Conditional removal with write_remove_if

```rust
let mut cursor = list.cursor();
let fill_qty = 10;

while let Some(guard) = cursor.next() {
    // Modify and conditionally remove
    if let Some(detached) = guard.write_remove_if(|order| {
        order.qty = order.qty.saturating_sub(fill_qty);
        order.qty == 0  // Remove if now empty
    }) {
        let node = detached.take(|order| index.remove(&order.id).unwrap());
        send_fill_report(node.take());
    }
}
```

---

## 11. Cursor Gap State (Bidirectional Safety)

After removal, cursor is in `Gap { prev, next }` state. This ensures both
forward and reverse iteration work correctly.

```rust
let mut cursor = list.cursor();

// Forward: A -> B -> C -> D
// We're at B, remove it
// Gap { prev: A, next: C }

if let Some(guard) = cursor.next() {  // At A
    guard.skip();
}
if let Some(guard) = cursor.next() {  // At B
    let detached = guard.remove();    // Remove B, cursor in Gap
    detached.try_take(|o| index.remove(&o.id));
}

// cursor.next() returns C (from Gap.next)
if let Some(guard) = cursor.next() {
    guard.read(|o| assert_eq!(o.id, C_ID));
}

// If we had called cursor.prev() instead, it would return A (from Gap.prev)
```

### Mixed direction after removal

```rust
let mut cursor = list.cursor();

// Navigate to middle
cursor.next();  // A
cursor.next();  // B
if let Some(guard) = cursor.next() {  // C
    guard.remove();  // Remove C, Gap { prev: B, next: D }
}

// Can go either direction from Gap:
cursor.next();  // -> D
cursor.prev();  // -> D (we're at D now)
cursor.prev();  // -> B (skipped the gap where C was)
```

---

## 12. Aliasing Prevention Summary

### Why slots are opaque (no get() method)

If `ListSlot` had `get()`:
```rust
// HYPOTHETICAL BAD API - we don't have this!
let slot = index.get(&1001).unwrap();
let order_ref: &Order = slot.get();  // Hypothetical

list.write(slot, |order| {
    order.qty -= 1;  // Mutating through list
});

println!("{}", order_ref.qty);  // UB: order_ref aliases the mutated data!
```

Our design prevents this:
```rust
let slot = index.get(&1001).unwrap();
// slot.get() doesn't exist!
// The ONLY way to access data is through list.read() or list.write()

let qty = list.read(slot, |order| order.qty);  // Returns u32, not &u32
// No reference escapes
```

### Why write() takes &mut slot

```rust
// write() signature:
// pub fn write<F, R>(&mut self, slot: &mut ListSlot<T>, f: F) -> R

let slot = index.get_mut(&1001).unwrap();

list.write(slot, |order| {
    // At this point:
    // - &mut list is borrowed (no other list operations possible)
    // - &mut slot is borrowed (can't use slot for anything else)
    // - |order| has &mut Order (exclusive access to data)

    // COMPILE ERROR: slot is borrowed
    // let _ = slot.anything();

    // COMPILE ERROR: list is borrowed
    // list.read(slot, |o| o.price);

    order.qty -= 1;
});
```

### Why Detached borrows the list

```rust
let detached: Detached<'_, Order> = list.pop_front().unwrap();
// detached borrows list mutably (lifetime tied to list)

// COMPILE ERROR: list is borrowed
// list.front(|o| o.id);

// Must consume detached first
let node = detached.take(|o| index.remove(&o.id).unwrap());

// Now list is free
list.front(|o| println!("{}", o.id));  // OK
```

---

## 13. Complete Workflow Example

```rust
use nexus_collections::{List, DetachedListNode, ListSlot, Node};
use nexus_slab::Slab;
use std::collections::HashMap;

struct Order {
    id: u64,
    price: f64,
    qty: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup
    let slab: Slab<Node<Order>> = Slab::with_capacity(1000);
    let mut list: List<Order, _> = List::new(&slab);
    let mut index: HashMap<u64, ListSlot<Order>> = HashMap::new();

    // Insert some orders
    for i in 1..=5 {
        let order = Order { id: i, price: 100.0 + i as f64, qty: 10 * i as u32 };
        let id = order.id;
        let detached = DetachedListNode::new(&slab, order)?;
        let slot = list.link_back(detached);
        index.insert(id, slot);
    }

    // Read front
    list.front(|order| {
        println!("Front order: {} @ {}", order.id, order.price);
    });

    // Modify order 3
    if let Some(slot) = index.get_mut(&3) {
        list.write(slot, |order| {
            order.qty += 100;
            println!("Updated order 3 qty to {}", order.qty);
        });
    }

    // Cancel order 2 (unlink and drop)
    if let Some(slot) = index.remove(&2) {
        let detached = list.unlink(slot);
        let order = detached.take();
        println!("Cancelled order {}", order.id);
    }

    // Move order 4 to front
    if let Some(slot) = index.remove(&4) {
        let detached = list.unlink(slot);
        let new_slot = list.link_front(detached);
        index.insert(4, new_slot);
        println!("Moved order 4 to front");
    }

    // Process orders with cursor, remove if qty < 30
    let mut cursor = list.cursor();
    while let Some(guard) = cursor.next() {
        if let Some(detached) = guard.read_remove_if(|order| {
            println!("Checking order {}: qty={}", order.id, order.qty);
            order.qty < 30
        }) {
            if let Some(node) = detached.try_take(|order| index.remove(&order.id)) {
                let order = node.take();
                println!("Removed order {} (qty {} < 30)", order.id, order.qty);
            }
        }
    }

    // Pop remaining orders
    while let Some(detached) = list.pop_front() {
        let node = detached.try_take(|order| index.remove(&order.id));
        if let Some(node) = node {
            let order = node.take();
            println!("Final: order {} with qty {}", order.id, order.qty);
        }
    }

    println!("Done. Index empty: {}", index.is_empty());
    Ok(())
}
```

---

## Safety Properties Summary

### Compile-Time (Borrow Checker Enforced)

| Property | Mechanism |
|----------|-----------|
| No aliasing during write | `write(&mut self, &mut slot, f)` borrows both |
| No reference escape | Closures return `R`, not `&T` |
| No double-link | Type-state: `link()` consumes `DetachedListNode` |
| No double-unlink | Type-state: `unlink()` consumes `ListSlot` |
| No use-after-pop | `Detached` borrows list until consumed |

### Runtime Checks

| Check | Safe API | Unchecked API |
|-------|----------|---------------|
| Slot validity (stale slot) | Panic with message | **UB** |
| Owner ID (wrong list) | `debug_assert` | `debug_assert` |

### User Invariants

| Invariant | Safe API | Unchecked API |
|-----------|----------|---------------|
| Consume all guards | Leak | Leak |
| Unlink slots before drop | Panic on next access | **UB** |
| Return correct slot from `take()` | Corruption | Corruption |
| Keep index in sync | Panic or wrong data | **UB** or wrong data |

**Safe API:** Violations cause panic with clear error message. No UB.

**Unchecked API:** Violations cause UB. Use only when you've verified invariants
and need maximum performance.
