use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nexus_async_rt::{DefaultBoundedAlloc, Executor};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

fn spawn_poll_immediate(c: &mut Criterion) {
    let mut executor = Executor::new(DefaultBoundedAlloc::new(64), 64);
    c.bench_function("spawn+poll immediate", |b| {
        b.iter(|| {
            executor.spawn(async { black_box(42u64); });
            executor.poll();
        });
    });
}

fn poll_stable_task(c: &mut Criterion) {
    struct PersistentTask;
    impl Future for PersistentTask {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            black_box(());
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    let mut executor = Executor::new(DefaultBoundedAlloc::new(4), 4);
    executor.spawn(PersistentTask);

    c.bench_function("poll stable task (self-wake)", |b| {
        b.iter(|| {
            executor.poll();
        });
    });
}

criterion_group!(benches, spawn_poll_immediate, poll_stable_task);
criterion_main!(benches);
