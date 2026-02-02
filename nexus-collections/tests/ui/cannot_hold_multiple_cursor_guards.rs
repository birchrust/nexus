//! Test that you cannot hold multiple cursor guards simultaneously.
//! Each guard borrows the cursor mutably, preventing multiple guards.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let d1 = test_alloc::create_node(1).unwrap();
    let d2 = test_alloc::create_node(2).unwrap();

    let _slot1 = list.link_back(d1);
    let _slot2 = list.link_back(d2);

    let mut cursor = list.cursor();

    // Get first guard
    let guard1 = cursor.next().unwrap();

    // Try to get second guard while holding first
    // ERROR: cannot borrow `cursor` as mutable more than once at a time
    let guard2 = cursor.next().unwrap();

    // Try to use both
    let _ = guard1.read(|x| *x);
    let _ = guard2.read(|x| *x);
}
