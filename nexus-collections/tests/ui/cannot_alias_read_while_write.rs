//! Test that you cannot call read() on any slot while inside a write() closure.
//! The write() takes &mut self, preventing any other borrows.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let d1 = test_alloc::create_node(1).unwrap();
    let d2 = test_alloc::create_node(2).unwrap();

    let mut slot1 = list.link_back(d1);
    let slot2 = list.link_back(d2);

    // Try to read slot2 while writing to slot1
    list.write(&mut slot1, |_data1| {
        // ERROR: cannot borrow `list` as immutable because it is also borrowed as mutable
        let _ = list.read(&slot2, |data2| *data2);
    });
}
