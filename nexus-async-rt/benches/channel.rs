use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nexus_async_rt::channel::local;
use nexus_async_rt::{Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;
use std::cell::Cell;
use std::rc::Rc;

// Sync path benchmarks omitted — they require channel creation outside
// block_on which conflicts with the runtime context assertion. The sync
// path was measured at ~7 cycles/op in unit test timing.

// =============================================================================
// Async path: measure full executor dispatch + channel ops
// =============================================================================

fn async_send_recv(c: &mut Criterion) {
    c.bench_function("local async send+recv 10k (buf=64)", |b| {
        let wb = WorldBuilder::new();
        let mut world = wb.build();
        let mut rt = Runtime::new(&mut world);

        b.iter(|| {
            rt.block_on(async {
                let (tx, rx) = local::channel::<u64>(64);

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
                assert_eq!(n, 10_000);
            });
        });
    });
}

fn async_send_recv_small_buffer(c: &mut Criterion) {
    c.bench_function("local async send+recv 10k (buf=4, backpressure)", |b| {
        let wb = WorldBuilder::new();
        let mut world = wb.build();
        let mut rt = Runtime::new(&mut world);

        b.iter(|| {
            rt.block_on(async {
                let (tx, rx) = local::channel::<u64>(4);

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
                assert_eq!(n, 10_000);
            });
        });
    });
}

criterion_group!(benches, async_send_recv, async_send_recv_small_buffer,);
criterion_main!(benches);
