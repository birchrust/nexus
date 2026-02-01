//! Test that link_back() requires DetachedListNode, not ListSlot.
//! Type-state prevents linking an already-linked slot.

use nexus_collections::{List, BoundedListSlab};

fn main() {
    let slab = BoundedListSlab::<u64>::with_capacity(16);
    let mut list: List<u64, _> = List::new(slab);

    let detached = slab.create_node(42).unwrap();
    let slot = list.link_back(detached);

    // ERROR: expected `DetachedListNode`, found `ListSlot`
    list.link_back(slot);
}
