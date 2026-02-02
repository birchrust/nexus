//! Test that detached node cannot be used after being moved to link_back().
//! The detached node is consumed by link_back(), preventing use-after-move.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();
    let _slot = list.link_back(detached);

    // ERROR: use of moved value: `detached`
    let _ = detached.take();
}
