//! Test that ListSlot is opaque - no get() method exists.
//! All data access must go through list.read() or list.write().

use nexus_collections::{List, BoundedListSlab};

fn main() {
    let slab = BoundedListSlab::<u64>::with_capacity(16);
    let mut list: List<u64, _> = List::new(slab);

    let detached = slab.create_node(42).unwrap();
    let slot = list.link_back(detached);

    // ERROR: no method named `get` found for struct `ListSlot`
    let _ = slot.get();
}
