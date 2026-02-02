//! Test that ListSlot is opaque - no get() method exists.
//! All data access must go through list.read() or list.write().

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();
    let slot = list.link_back(detached);

    // ERROR: no method named `get` found for struct `ListSlot`
    let _ = slot.get();
}
