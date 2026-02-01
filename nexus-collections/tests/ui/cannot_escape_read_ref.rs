//! Test that references cannot escape from read() closure.

use nexus_collections::{List, BoundedListSlab};

fn main() {
    let slab = BoundedListSlab::<u64>::with_capacity(16);
    let mut list: List<u64, _> = List::new(slab);

    let detached = slab.create_node(42).unwrap();
    let slot = list.link_back(detached);

    let escaped: &u64;
    list.read(&slot, |data| {
        escaped = data;  // ERROR: reference escapes closure
    });
}
