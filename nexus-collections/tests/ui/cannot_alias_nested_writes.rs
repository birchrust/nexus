//! Test that you cannot have nested write() calls.
//! This would create two &mut T references to different slots simultaneously.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let d1 = test_alloc::create_node(1).unwrap();
    let d2 = test_alloc::create_node(2).unwrap();

    let mut slot1 = list.link_back(d1);
    let mut slot2 = list.link_back(d2);

    // Try to write to slot2 while writing to slot1
    list.write(&mut slot1, |_data1| {
        // ERROR: cannot borrow `list` as mutable more than once at a time
        list.write(&mut slot2, |_data2| {});
    });
}
