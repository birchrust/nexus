//! Test that references cannot escape from write() closure.

use nexus_collections::create_list;

create_list!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();
    let mut slot = list.link_back(detached);

    let escaped: &mut u64;
    list.write(&mut slot, |data| {
        escaped = data;  // ERROR: reference escapes closure
    });
}
