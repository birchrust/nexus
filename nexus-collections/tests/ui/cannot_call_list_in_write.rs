//! Test that list methods cannot be called inside write() closure.
//! The &mut self borrow prevents any other access to the list.

use nexus_collections::create_list;

create_list!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();
    let mut slot = list.link_back(detached);

    list.write(&mut slot, |_data| {
        // ERROR: cannot borrow `list` as immutable because it is also borrowed as mutable
        let _ = list.len();
    });
}
