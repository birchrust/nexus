//! Test that detached node cannot be used after being moved to link_back().
//! The detached node is consumed by link_back(), preventing use-after-move.

use nexus_collections::{List, BoundedListSlab};

fn main() {
    let slab = BoundedListSlab::<u64>::with_capacity(16);
    let mut list: List<u64, _> = List::new(slab);

    let detached = slab.create_node(42).unwrap();
    let _slot = list.link_back(detached);

    // ERROR: use of moved value: `detached`
    let _ = detached.take();
}
