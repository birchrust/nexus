//! Test that link_back() requires DetachedListNode, not ListSlot.
//! Type-state prevents linking an already-linked slot.

use nexus_collections::create_list;

create_list!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();
    let slot = list.link_back(detached);

    // ERROR: expected `DetachedListNode`, found `ListSlot`
    list.link_back(slot);
}
