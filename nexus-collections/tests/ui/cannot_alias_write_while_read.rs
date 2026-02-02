//! Test that you cannot call write() while inside a read() closure.
//! Even though read() only takes &self, calling write() requires &mut self,
//! which conflicts with the existing &self borrow.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let d1 = test_alloc::create_node(1).unwrap();
    let d2 = test_alloc::create_node(2).unwrap();

    let slot1 = list.link_back(d1);
    let mut slot2 = list.link_back(d2);

    // Try to write to slot2 while reading slot1
    list.read(&slot1, |_data1| {
        // ERROR: cannot borrow `list` as mutable because it is also borrowed as immutable
        list.write(&mut slot2, |data2| {
            *data2 = 99;
        });
    });
}
