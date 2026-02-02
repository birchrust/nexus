//! Test that you cannot use list methods while a cursor is active.
//! The cursor holds &mut list, preventing other borrows.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let d1 = test_alloc::create_node(1).unwrap();
    let d2 = test_alloc::create_node(2).unwrap();

    let slot1 = list.link_back(d1);
    let _slot2 = list.link_back(d2);

    // Create cursor (borrows &mut list)
    let mut cursor = list.cursor();

    // Try to read via list while cursor exists
    // ERROR: cannot borrow `list` as immutable because it is also borrowed as mutable
    let _ = list.read(&slot1, |x| *x);

    // Use cursor to show it's still active
    let _ = cursor.next();
}
