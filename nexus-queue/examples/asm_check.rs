use std::hint::black_box;

#[repr(C)]
struct Order {
    id: u64,
    price: f64,
    qty: f64,
}

fn main() {
    let (producer, consumer) = nexus_queue::spsc::ring_buffer::<Order>(1024);

    for _ in 0..1000 {
        let _ = black_box(&producer).push(Order {
            id: 1,
            price: 100.5,
            qty: 10.0,
        });
        if let Some(val) = black_box(&consumer).pop() {
            black_box(&val);
        }
    }
}
