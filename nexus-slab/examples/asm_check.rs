use std::hint::black_box;

#[repr(C)]
struct Order {
    id: u64,
    price: f64,
    qty: f64,
}

fn main() {
    // SAFETY: test binary, single slab
    let slab = unsafe { nexus_slab::bounded::Slab::<Order>::with_capacity(1024) };

    for _ in 0..1000 {
        let ptr = black_box(&slab).alloc(Order { id: 1, price: 100.5, qty: 10.0 });
        black_box(&*ptr);
        black_box(&slab).free(ptr);
    }
}
