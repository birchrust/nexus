//! Test that references cannot escape from read() closure.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();
    let slot = list.link_back(detached);

    let escaped: &u64;
    list.read(&slot, |data| {
        escaped = data;  // ERROR: reference escapes closure
    });
}
