use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nexus_async_rt::channel::mpsc;

// =============================================================================
// Synchronous path benchmarks (no executor needed)
// =============================================================================

fn try_send_try_recv(c: &mut Criterion) {
    let (tx, rx) = mpsc::channel::<u64>(64);

    c.bench_function("mpsc try_send+try_recv", |b| {
        b.iter(|| {
            tx.try_send(black_box(42u64)).unwrap();
            let v = rx.try_recv().unwrap();
            black_box(v);
        });
    });
}

fn try_send_burst_then_drain(c: &mut Criterion) {
    let (tx, rx) = mpsc::channel::<u64>(64);

    c.bench_function("mpsc burst 64 send + 64 recv", |b| {
        b.iter(|| {
            for i in 0..64u64 {
                tx.try_send(black_box(i)).unwrap();
            }
            for _ in 0..64 {
                black_box(rx.try_recv().unwrap());
            }
        });
    });
}

fn try_send_throughput(c: &mut Criterion) {
    let (tx, rx) = mpsc::channel::<u64>(1024);

    c.bench_function("mpsc 10k try_send+try_recv sequential", |b| {
        b.iter(|| {
            for i in 0..10_000u64 {
                tx.try_send(i).unwrap();
                black_box(rx.try_recv().unwrap());
            }
        });
    });
}

// =============================================================================
// Async path benchmarks (using the real executor)
// =============================================================================

fn async_send_recv(c: &mut Criterion) {
    use nexus_async_rt::{Runtime, spawn_boxed};
    use nexus_rt::WorldBuilder;
    use std::cell::Cell;
    use std::rc::Rc;

    c.bench_function("mpsc async send+recv 10k (real executor)", |b| {
        b.iter(|| {
            let wb = WorldBuilder::new();
            let mut world = wb.build();
            let mut rt = Runtime::new(&mut world);
            let count = Rc::new(Cell::new(0u64));
            let count_clone = count.clone();

            rt.block_on(async move {
                let (tx, rx) = mpsc::channel::<u64>(64);

                spawn_boxed(async move {
                    for i in 0..10_000u64 {
                        tx.send(i).await.unwrap();
                    }
                });

                let mut n = 0u64;
                loop {
                    match rx.recv().await {
                        Ok(v) => {
                            black_box(v);
                            n += 1;
                        }
                        Err(_) => break,
                    }
                }
                count_clone.set(n);
            });
            assert_eq!(count.get(), 10_000);
        });
    });
}

fn async_send_recv_small_buffer(c: &mut Criterion) {
    use nexus_async_rt::{Runtime, spawn_boxed};
    use nexus_rt::WorldBuilder;
    use std::cell::Cell;
    use std::rc::Rc;

    c.bench_function("mpsc async send+recv 10k (buf=4, backpressure)", |b| {
        b.iter(|| {
            let wb = WorldBuilder::new();
            let mut world = wb.build();
            let mut rt = Runtime::new(&mut world);
            let count = Rc::new(Cell::new(0u64));
            let count_clone = count.clone();

            rt.block_on(async move {
                let (tx, rx) = mpsc::channel::<u64>(4);

                spawn_boxed(async move {
                    for i in 0..10_000u64 {
                        tx.send(i).await.unwrap();
                    }
                });

                let mut n = 0u64;
                loop {
                    match rx.recv().await {
                        Ok(v) => {
                            black_box(v);
                            n += 1;
                        }
                        Err(_) => break,
                    }
                }
                count_clone.set(n);
            });
            assert_eq!(count.get(), 10_000);
        });
    });
}

criterion_group!(
    benches,
    try_send_try_recv,
    try_send_burst_then_drain,
    try_send_throughput,
    async_send_recv,
    async_send_recv_small_buffer,
);
criterion_main!(benches);
