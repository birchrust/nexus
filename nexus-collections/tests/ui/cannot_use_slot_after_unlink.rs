//! Test that slot cannot be used after being moved to unlink().
//! The slot is consumed by unlink(), preventing use-after-move.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();
    let slot = list.link_back(detached);

    let _detached = list.unlink(slot);

    // ERROR: use of moved value: `slot`
    let _ = list.read(&slot, |x| *x);
}
