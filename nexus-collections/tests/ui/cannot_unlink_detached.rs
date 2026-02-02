//! Test that unlink() requires ListSlot, not DetachedListNode.
//! Type-state prevents unlinking a not-linked node.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();

    // ERROR: expected `ListSlot`, found `DetachedListNode`
    list.unlink(detached);
}
