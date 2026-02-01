//! Test that unlink() requires ListSlot, not DetachedListNode.
//! Type-state prevents unlinking a not-linked node.

use nexus_collections::{List, BoundedListSlab};

fn main() {
    let slab = BoundedListSlab::<u64>::with_capacity(16);
    let mut list: List<u64, _> = List::new(slab);

    let detached = slab.create_node(42).unwrap();

    // ERROR: expected `ListSlot`, found `DetachedListNode`
    list.unlink(detached);
}
