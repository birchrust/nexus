//! Test that references cannot escape from cursor guard write() closure.

use nexus_collections::list_allocator;

list_allocator!(test_alloc, u64);

fn main() {
    test_alloc::init().bounded(16).build();
    let mut list = test_alloc::List::new();

    let detached = test_alloc::create_node(42).unwrap();
    let _slot = list.link_back(detached);

    let escaped: &mut u64;
    {
        let mut cursor = list.cursor();
        let guard = cursor.next().unwrap();
        guard.write(|data| {
            escaped = data; // ERROR: reference escapes closure
        });
    }
}
