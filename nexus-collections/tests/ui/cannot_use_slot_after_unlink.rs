//! Test that slot cannot be used after being moved to unlink().
//! The slot is consumed by unlink(), preventing use-after-move.

use nexus_collections::{List, BoundedListSlab};

fn main() {
    let slab = BoundedListSlab::<u64>::with_capacity(16);
    let mut list: List<u64, _> = List::new(slab);

    let detached = slab.create_node(42).unwrap();
    let slot = list.link_back(detached);

    let _detached = list.unlink(slot);

    // ERROR: use of moved value: `slot`
    let _ = list.read(&slot, |x| *x);
}
