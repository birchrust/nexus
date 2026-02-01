//! Test that list methods cannot be called inside write() closure.
//! The &mut self borrow prevents any other access to the list.

use nexus_collections::{List, BoundedListSlab};

fn main() {
    let slab = BoundedListSlab::<u64>::with_capacity(16);
    let mut list: List<u64, _> = List::new(slab);

    let detached = slab.create_node(42).unwrap();
    let mut slot = list.link_back(detached);

    list.write(&mut slot, |_data| {
        // ERROR: cannot borrow `list` as immutable because it is also borrowed as mutable
        let _ = list.len();
    });
}
