//! Test that you cannot use a cursor while holding a reference from list.read().
//! The cursor requires &mut list, which conflicts with the &list borrow from read().

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let d1 = test_alloc::create_node(1).unwrap();
    let d2 = test_alloc::create_node(2).unwrap();

    let slot1 = list.link_back(d1);
    let _slot2 = list.link_back(d2);

    // Try to create cursor while inside read() closure
    list.read(&slot1, |_data1| {
        // ERROR: cannot borrow `list` as mutable because it is also borrowed as immutable
        let mut cursor = list.cursor();
        let _ = cursor.next();
    });
}
