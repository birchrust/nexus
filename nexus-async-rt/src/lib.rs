//! Single-threaded async runtime with pre-allocated task storage.
//!
//! Zero-allocation task spawn via nexus-slab byte slab (placement new).
//! Zero-allocation wakers (raw pointer as data, no Box, no Arc).
//! Designed for the hot path.
//!
//! ```ignore
//! use nexus_async_rt::{Executor, ExecutorBuilder};
//!
//! // Bounded: fixed capacity, panics if full
//! let mut executor = ExecutorBuilder::<256>::bounded(64);
//!
//! // Unbounded: grows via chunks, never fails
//! let mut executor = ExecutorBuilder::<256>::unbounded(64);
//!
//! executor.spawn(async {
//!     let data = read_socket().await;
//!     process(data);
//! });
//!
//! executor.poll();
//! ```

mod task;
mod waker;
mod world_ctx;
mod io;
mod timer;
mod runtime;

pub use task::{TaskId, TASK_HEADER_SIZE};
pub use world_ctx::WorldCtx;
pub use io::IoHandle;
pub use timer::{Sleep, TimerHandle};
pub use runtime::{DefaultRuntime, Runtime, RuntimeHandle, spawn};

use std::future::Future;
use std::task::Poll;

use task::Task;
use waker::{set_ready_queue, ReusableWaker};

// =============================================================================
// Byte slab backend
// =============================================================================

/// Allocation strategy for task storage.
enum SlabBackend<const INTERNAL_SIZE: usize> {
    Bounded(nexus_slab::byte::bounded::Slab<INTERNAL_SIZE>),
    Unbounded(nexus_slab::byte::unbounded::Slab<INTERNAL_SIZE>),
}

impl<const N: usize> SlabBackend<N> {
    #[inline]
    fn alloc_task<F: Future<Output = ()> + 'static>(&self, task: Task<F>) -> *mut u8 {
        match self {
            Self::Bounded(slab) => slab.alloc(task).into_raw(),
            Self::Unbounded(slab) => slab.alloc(task).into_raw(),
        }
    }

    #[inline]
    unsafe fn free_slot(&self, ptr: *mut u8) {
        // SAFETY: ptr was returned by alloc_task. The future inside has
        // already been dropped via drop_task_future. We reconstruct a
        // byte::Slot<u8> — the type doesn't matter for free since the
        // value is already dropped (u8 is Copy, no-op drop).
        match self {
            Self::Bounded(slab) => {
                let slot = unsafe { nexus_slab::byte::Slot::<u8>::from_raw(ptr) };
                slab.free(slot);
            }
            Self::Unbounded(slab) => {
                let slot = unsafe { nexus_slab::byte::Slot::<u8>::from_raw(ptr) };
                slab.free(slot);
            }
        }
    }
}

// =============================================================================
// ExecutorBuilder
// =============================================================================

/// Computes the internal slab slot size from user-visible future capacity.
///
/// The slab stores a task header (24 bytes) alongside each future.
/// Use this to compute the `INTERNAL_SIZE` const generic for [`Executor`]:
///
/// ```ignore
/// use nexus_async_rt::{Executor, slot_size};
///
/// // Executor with 256-byte future capacity (280-byte slab slots)
/// let executor = Executor::<{ slot_size(256) }>::with_capacity(64);
/// ```
pub const fn slot_size(future_capacity: usize) -> usize {
    future_capacity + TASK_HEADER_SIZE
}

// =============================================================================
// Executor
// =============================================================================

/// Single-threaded async executor with byte-slab task storage.
///
/// `INTERNAL_SIZE` is the slab slot size (future + header). Users
/// interact via [`ExecutorBuilder`] which adds header overhead
/// automatically, or via [`with_capacity`](Self::with_capacity).
pub struct Executor<const INTERNAL_SIZE: usize> {
    /// Byte slab: task header + future bytes in one slot.
    slab: SlabBackend<INTERNAL_SIZE>,

    /// Incoming ready tasks. Wakers and spawn push here.
    /// Swapped with `draining` at the start of each poll cycle.
    incoming: Vec<*mut u8>,

    /// Tasks being drained this cycle. Iterated linearly —
    /// perfect cache-line utilization, no index math.
    draining: Vec<*mut u8>,

    /// All live task pointers (including pending tasks not in any queue).
    /// Enables complete cleanup on Drop — no leaked futures.
    all_tasks: Vec<*mut u8>,

    /// Number of live tasks.
    live_count: usize,
}

impl<const INTERNAL_SIZE: usize> Executor<INTERNAL_SIZE> {
    /// Create a bounded executor with fixed task capacity.
    ///
    /// `INTERNAL_SIZE` includes task header overhead (24 bytes). Use
    /// [`slot_size`] to compute from your max future size:
    ///
    /// ```ignore
    /// let executor = Executor::<{ slot_size(256) }>::with_capacity(64);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        // SAFETY: Single-threaded use. Slot lifetimes managed by Executor.
        let slab = unsafe {
            nexus_slab::byte::bounded::Slab::with_capacity(capacity)
        };
        Self {
            slab: SlabBackend::Bounded(slab),
            incoming: Vec::with_capacity(capacity),
            draining: Vec::with_capacity(capacity),
            all_tasks: Vec::with_capacity(capacity),
            live_count: 0,
        }
    }

    /// Create an unbounded executor that grows as needed.
    ///
    /// `initial_chunk_capacity` is the number of task slots in the
    /// first chunk. Grows via independent chunks — no reallocation.
    pub fn unbounded(initial_chunk_capacity: usize) -> Self {
        // SAFETY: Single-threaded use. Slot lifetimes managed by Executor.
        let slab = unsafe {
            nexus_slab::byte::unbounded::Slab::with_chunk_capacity(initial_chunk_capacity)
        };
        Self {
            slab: SlabBackend::Unbounded(slab),
            incoming: Vec::with_capacity(initial_chunk_capacity),
            draining: Vec::with_capacity(initial_chunk_capacity),
            all_tasks: Vec::with_capacity(initial_chunk_capacity),
            live_count: 0,
        }
    }

    /// Spawn an async task. Returns its [`TaskId`].
    ///
    /// The task (header + future) is stored via placement new into the
    /// byte slab — no intermediate copy. Panics if the task exceeds
    /// the slab slot size.
    ///
    /// The task is immediately queued for its first poll.
    pub fn spawn<F>(&mut self, future: F) -> TaskId
    where
        F: Future<Output = ()> + 'static,
    {
        let task = Task::new(future);

        // Placement new into the byte slab.
        let ptr = self.slab.alloc_task(task);

        // Track for cleanup on Drop.
        self.all_tasks.push(ptr);

        // Mark as queued and push to incoming queue.
        // SAFETY: ptr points to a live Task we just allocated.
        unsafe { task::set_queued(ptr, true) };
        self.incoming.push(ptr);
        self.live_count += 1;

        TaskId(ptr)
    }

    /// Poll all ready tasks once.
    ///
    /// Drains the ready queue and polls each task. Tasks that return
    /// `Pending` stay in the slab — their waker will re-queue them.
    /// Tasks that return `Ready` are dropped and their slots freed.
    ///
    /// Returns the number of tasks that completed this cycle.
    pub fn poll(&mut self) -> usize {
        let mut completed = 0;

        // Double-buffer swap: move incoming tasks to draining, leave
        // incoming empty for wakers to push to during this cycle.
        std::mem::swap(&mut self.incoming, &mut self.draining);

        // Install TLS ready queue pointing to incoming — wakers and
        // timers push here during this poll cycle.
        let _guard = set_ready_queue(&mut self.incoming);

        // Pre-built waker + context: vtable and Context layout set once,
        // only data pointer updated per task.
        let mut reusable = ReusableWaker::new();
        reusable.init();

        // Linear drain — sequential memory access, no index math.
        for &ptr in &self.draining {
            // Clear queued flag so the task can be re-queued by its waker.
            // SAFETY: ptr points to a live Task in the slab.
            unsafe { task::set_queued(ptr, false) };

            // Update the reusable waker+context with this task's pointer.
            // SAFETY: ptr is valid for the task's lifetime. ReusableWaker
            // is on the stack and not moved after init.
            let cx = unsafe { reusable.set_task(ptr) };

            // SAFETY: ptr points to a live Task. Future bytes are at
            // a fixed offset. Slab memory is stable (Pin sound).
            let poll_result = unsafe { task::poll_task(ptr, cx) };

            match poll_result {
                Poll::Pending => {}
                Poll::Ready(()) => {
                    // Drop the future and free the slab slot.
                    // SAFETY: future is live, single drop.
                    unsafe { task::drop_task_future(ptr) };
                    // SAFETY: ptr was returned by alloc_task.
                    unsafe { self.slab.free_slot(ptr) };
                    // Remove from all_tasks tracking.
                    if let Some(pos) = self.all_tasks.iter().position(|p| *p == ptr) {
                        self.all_tasks.swap_remove(pos);
                    }
                    self.live_count -= 1;
                    completed += 1;
                }
            }
        }

        // Clear draining buffer, keep capacity for next cycle.
        self.draining.clear();

        completed
    }

    /// Number of live tasks.
    pub fn task_count(&self) -> usize {
        self.live_count
    }

    /// Returns `true` if any tasks are queued for polling.
    pub fn has_ready(&self) -> bool {
        !self.incoming.is_empty()
    }

    /// Returns a mutable reference to the incoming queue.
    /// Used by Runtime to install TLS covering timers + IO.
    pub(crate) fn ready_queue_mut(&mut self) -> &mut Vec<*mut u8> {
        &mut self.incoming
    }

    /// Run the executor until all tasks complete.
    pub fn block_on(&mut self) {
        while self.task_count() > 0 {
            if self.has_ready() {
                self.poll();
            } else {
                std::thread::yield_now();
            }
        }
    }

    /// Cancel a task by ID. The future is dropped immediately.
    ///
    /// Must not be called while `poll()` is executing (enforced by
    /// `&mut self` — `poll` also takes `&mut self`).
    pub fn cancel(&mut self, id: TaskId) {
        let ptr = id.0;
        // SAFETY: ptr points to a live Task.
        unsafe { task::drop_task_future(ptr) };
        // SAFETY: ptr was returned by alloc_task.
        unsafe { self.slab.free_slot(ptr) };
        self.live_count -= 1;

        // Remove from all tracking structures.
        self.incoming.retain(|p| *p != ptr);
        self.draining.retain(|p| *p != ptr);
        if let Some(pos) = self.all_tasks.iter().position(|p| *p == ptr) {
            self.all_tasks.swap_remove(pos);
        }
    }
}

impl<const INTERNAL_SIZE: usize> Drop for Executor<INTERNAL_SIZE> {
    fn drop(&mut self) {
        // Drop ALL live tasks — including pending ones not in any queue.
        // This ensures futures' Drop impls run (releasing file descriptors,
        // Rc references, slab slots in other allocators, etc.).
        for &ptr in &self.all_tasks {
            // SAFETY: ptr points to a live Task. Each pointer appears
            // exactly once in all_tasks (inserted at spawn, removed at
            // completion/cancel).
            unsafe { task::drop_task_future(ptr) };
            unsafe { self.slab.free_slot(ptr) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;
    use std::pin::Pin;
    use std::task::Context;

    // Helper: bounded executor with enough headroom for test futures.
    fn test_executor() -> Executor<{ 128 + TASK_HEADER_SIZE }> {
        Executor::with_capacity(16)
    }

    #[test]
    fn spawn_and_poll_immediate() {
        let mut executor = test_executor();

        let mut completed = false;
        let flag = &mut completed as *mut bool;

        executor.spawn(async move {
            // SAFETY: test-only, single-threaded.
            unsafe { *flag = true };
        });

        assert_eq!(executor.task_count(), 1);
        let done = executor.poll();
        assert_eq!(done, 1);
        assert!(completed);
        assert_eq!(executor.task_count(), 0);
    }

    #[test]
    fn spawn_and_poll_with_yields() {
        use std::cell::Cell;
        use std::rc::Rc;

        let counter = Rc::new(Cell::new(0u32));
        let mut executor = test_executor();

        struct YieldOnce(bool);
        impl Future for YieldOnce {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.0 {
                    Poll::Ready(())
                } else {
                    self.0 = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }

        let c = counter.clone();
        executor.spawn(async move {
            c.set(c.get() + 1);
            YieldOnce(false).await;
            c.set(c.get() + 1);
            YieldOnce(false).await;
            c.set(c.get() + 1);
        });

        assert_eq!(executor.poll(), 0);
        assert_eq!(counter.get(), 1);

        assert_eq!(executor.poll(), 0);
        assert_eq!(counter.get(), 2);

        assert_eq!(executor.poll(), 1);
        assert_eq!(counter.get(), 3);
        assert_eq!(executor.task_count(), 0);
    }

    #[test]
    fn multiple_tasks() {
        use std::cell::Cell;
        use std::rc::Rc;

        let counter = Rc::new(Cell::new(0u32));
        let mut executor = test_executor();

        for _ in 0..3 {
            let c = counter.clone();
            executor.spawn(async move {
                c.set(c.get() + 1);
            });
        }

        assert_eq!(executor.task_count(), 3);
        let done = executor.poll();
        assert_eq!(done, 3);
        assert_eq!(counter.get(), 3);
        assert_eq!(executor.task_count(), 0);
    }

    #[test]
    fn cancel_task() {
        let mut executor = test_executor();

        struct NeverReady;
        impl Future for NeverReady {
            type Output = ();
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Pending
            }
        }

        let id = executor.spawn(NeverReady);
        assert_eq!(executor.task_count(), 1);

        executor.poll();
        assert_eq!(executor.task_count(), 1);

        executor.cancel(id);
        assert_eq!(executor.task_count(), 0);
    }

    #[test]
    #[should_panic(expected = "exceeds byte slab slot size")]
    fn spawn_oversized_panics() {
        let big = [0u64; 128]; // 1024 bytes
        let mut executor = test_executor(); // 128 + 24 byte slots
        executor.spawn(async move {
            std::hint::black_box(&big);
            std::future::pending::<()>().await;
        });
    }

    #[test]
    fn task_size_reporting() {
        async fn small_task() {}
        async fn medium_task() {
            let _buf = [0u8; 64];
            std::future::pending::<()>().await;
        }

        println!(
            "small_task future: {} bytes (+ {} header = {} total)",
            std::mem::size_of_val(&small_task()),
            TASK_HEADER_SIZE,
            std::mem::size_of_val(&small_task()) + TASK_HEADER_SIZE,
        );
        println!(
            "medium_task future: {} bytes (+ {} header = {} total)",
            std::mem::size_of_val(&medium_task()),
            TASK_HEADER_SIZE,
            std::mem::size_of_val(&medium_task()) + TASK_HEADER_SIZE,
        );
    }

    // =========================================================================
    // Waker correctness
    // =========================================================================

    #[test]
    fn waker_dedup_prevents_double_queue() {
        use std::cell::Cell;
        use std::rc::Rc;

        let poll_count = Rc::new(Cell::new(0u32));
        let mut executor = test_executor();

        struct WakeThrice(Rc<Cell<u32>>);
        impl Future for WakeThrice {
            type Output = ();
            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                self.0.set(self.0.get() + 1);
                if self.0.get() >= 3 {
                    return Poll::Ready(());
                }
                // Wake multiple times — should only enqueue once.
                cx.waker().wake_by_ref();
                cx.waker().wake_by_ref();
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }

        executor.spawn(WakeThrice(poll_count.clone()));

        // First poll: poll_count=1, wakes 3x but should enqueue once.
        assert_eq!(executor.poll(), 0);
        assert_eq!(poll_count.get(), 1);

        // Second poll: exactly 1 task in queue (dedup worked).
        assert_eq!(executor.poll(), 0);
        assert_eq!(poll_count.get(), 2);

        // Third poll: completes.
        assert_eq!(executor.poll(), 1);
        assert_eq!(poll_count.get(), 3);
    }

    #[test]
    fn waker_clone_works() {
        use std::cell::Cell;
        use std::rc::Rc;

        let woken = Rc::new(Cell::new(false));
        let mut executor = test_executor();

        struct StoreAndWake {
            woken: Rc<Cell<bool>>,
            stored_waker: Option<std::task::Waker>,
        }
        impl Future for StoreAndWake {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.woken.get() {
                    return Poll::Ready(());
                }
                // Clone the waker and store it, then wake via the clone.
                let cloned = cx.waker().clone();
                self.stored_waker = Some(cloned);
                self.woken.set(true);
                // Wake via the stored clone.
                self.stored_waker.as_ref().unwrap().wake_by_ref();
                Poll::Pending
            }
        }

        executor.spawn(StoreAndWake {
            woken: woken.clone(),
            stored_waker: None,
        });

        assert_eq!(executor.poll(), 0); // Pending, waker cloned and fired
        assert!(woken.get());
        assert_eq!(executor.poll(), 1); // Ready
    }

    // =========================================================================
    // Task lifecycle — drop correctness
    // =========================================================================

    #[test]
    fn task_drop_on_completion() {
        use std::cell::Cell;
        use std::rc::Rc;

        let dropped = Rc::new(Cell::new(false));
        let mut executor = test_executor();

        struct DropTracker(Rc<Cell<bool>>);
        impl Drop for DropTracker {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let d = dropped.clone();
        executor.spawn(async move {
            let _guard = DropTracker(d);
            // Task completes — guard should be dropped.
        });

        assert!(!dropped.get());
        executor.poll();
        assert!(dropped.get(), "future not dropped on completion");
    }

    #[test]
    fn task_drop_on_cancel() {
        use std::cell::Cell;
        use std::rc::Rc;

        let dropped = Rc::new(Cell::new(false));
        let mut executor = test_executor();

        struct DropTracker(Rc<Cell<bool>>);
        impl Drop for DropTracker {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let d = dropped.clone();
        let id = executor.spawn(async move {
            let _guard = DropTracker(d);
            std::future::pending::<()>().await;
        });

        executor.poll(); // Pending
        assert!(!dropped.get());

        executor.cancel(id);
        assert!(dropped.get(), "future not dropped on cancel");
        assert_eq!(executor.task_count(), 0);
    }

    #[test]
    fn cancel_frees_slot_for_reuse() {
        let mut executor = test_executor();

        // Spawn and cancel repeatedly — slots should be reused.
        for _ in 0..100 {
            let id = executor.spawn(async {
                std::future::pending::<()>().await;
            });
            executor.poll();
            executor.cancel(id);
        }
        assert_eq!(executor.task_count(), 0);
    }

    #[test]
    fn executor_drop_cleans_up_queued_tasks() {
        use std::cell::Cell;
        use std::rc::Rc;

        let dropped = Rc::new(Cell::new(0u32));

        {
            let mut executor = test_executor();

            struct DropCounter(Rc<Cell<u32>>);
            impl Drop for DropCounter {
                fn drop(&mut self) {
                    self.0.set(self.0.get() + 1);
                }
            }

            for _ in 0..3 {
                // Capture the DropCounter — it's part of the future's state
                // and will be dropped when the executor drops the future.
                let guard = DropCounter(dropped.clone());
                executor.spawn(async move {
                    let _keep = guard;
                    std::future::pending::<()>().await;
                });
            }

            // Poll once so tasks go to Pending (still live in slab).
            executor.poll();
            assert_eq!(dropped.get(), 0);

            // Drop the executor — all_tasks tracking ensures ALL live
            // tasks are dropped, including pending ones.
        }

        assert_eq!(
            dropped.get(),
            3,
            "pending tasks not cleaned up on executor drop"
        );

        // Also verify the queued (never-polled) path works:
        // the queued (never-polled) path works:
        let dropped2 = Rc::new(Cell::new(0u32));
        {
            let mut executor = test_executor();

            struct DropCounter2(Rc<Cell<u32>>);
            impl Drop for DropCounter2 {
                fn drop(&mut self) {
                    self.0.set(self.0.get() + 1);
                }
            }

            for _ in 0..3 {
                let guard = DropCounter2(dropped2.clone());
                executor.spawn(async move {
                    let _keep = guard;
                    // Future body doesn't matter — executor drops before poll.
                });
            }

            // Don't poll — tasks are in the ready queue. Drop executor.
        }

        assert_eq!(
            dropped2.get(),
            3,
            "queued tasks not cleaned up on executor drop"
        );
    }

    // =========================================================================
    // Drain snapshot — no infinite loop
    // =========================================================================

    #[test]
    fn self_waking_task_does_not_infinite_loop() {
        use std::cell::Cell;
        use std::rc::Rc;

        let poll_count = Rc::new(Cell::new(0u32));
        let mut executor = test_executor();

        struct SelfWaker(Rc<Cell<u32>>);
        impl Future for SelfWaker {
            type Output = ();
            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                self.0.set(self.0.get() + 1);
                if self.0.get() >= 10 {
                    return Poll::Ready(());
                }
                cx.waker().wake_by_ref(); // re-queue every poll
                Poll::Pending
            }
        }

        executor.spawn(SelfWaker(poll_count.clone()));

        // Each poll() should advance by exactly 1 (drain snapshot prevents
        // re-polling in the same cycle).
        for expected in 1..10 {
            let completed = executor.poll();
            assert_eq!(poll_count.get(), expected);
            if expected < 10 {
                assert_eq!(completed, 0);
            }
        }
        // Final poll: completes.
        assert_eq!(executor.poll(), 1);
        assert_eq!(poll_count.get(), 10);
    }

    // =========================================================================
    // Unbounded executor
    // =========================================================================

    #[test]
    fn unbounded_grows_beyond_initial() {
        let mut executor =
            Executor::<{ 64 + TASK_HEADER_SIZE }>::unbounded(2);

        // Spawn well beyond initial chunk capacity.
        for _ in 0..50 {
            executor.spawn(async {});
        }
        assert_eq!(executor.task_count(), 50);
        let done = executor.poll();
        assert_eq!(done, 50);
    }

    // =========================================================================
    // Latency distribution benchmarks (run with --ignored --nocapture)
    //
    // cargo test -p nexus-async-rt --release -- --ignored --nocapture dispatch_latency
    // =========================================================================

    #[inline(always)]
    fn rdtsc() -> u64 {
        unsafe { core::arch::x86_64::_rdtsc() }
    }

    #[inline(always)]
    fn rdtscp() -> u64 {
        unsafe {
            let mut aux: u32 = 0;
            let tsc = core::arch::x86_64::__rdtscp(&mut aux);
            core::arch::x86_64::_mm_lfence();
            tsc
        }
    }

    fn print_distribution(name: &str, samples: &mut [u64]) {
        samples.sort_unstable();
        let len = samples.len();
        let p50 = samples[len / 2];
        let p90 = samples[len * 90 / 100];
        let p99 = samples[len * 99 / 100];
        let p999 = samples[len * 999 / 1000];
        let p9999 = samples[len * 9999 / 10000];
        let min = samples[0];
        let max = samples[len - 1];
        println!(
            "{name:<35} min:{min:>5}  p50:{p50:>5}  p90:{p90:>5}  p99:{p99:>5}  p999:{p999:>5}  p9999:{p9999:>5}  max:{max:>6}"
        );
    }

    const BENCH_WARMUP: usize = 100_000;
    const BENCH_SAMPLES: usize = 1_000_000;

    #[test]
    #[ignore]
    fn dispatch_latency_distribution() {
        use std::cell::Cell;
        use std::rc::Rc;

        let mut samples = Vec::with_capacity(BENCH_SAMPLES);

        println!("\n=== Dispatch Latency Distribution (rdtsc, all values in cycles) ===\n");

        const BATCH: usize = 100;

        // -- rdtsc floor --
        {
            samples.clear();
            for _ in 0..BENCH_WARMUP {
                let s = rdtsc();
                let e = rdtscp();
                black_box(e.wrapping_sub(s));
            }
            for _ in 0..BENCH_SAMPLES {
                let s = rdtsc();
                let e = rdtscp();
                samples.push(e.wrapping_sub(s));
            }
            print_distribution("rdtsc/rdtscp floor (per-sample)", &mut samples);
        }

        println!();

        // -- Sync mono handler --
        {
            use nexus_rt::{Handler, IntoHandler, ResMut, WorldBuilder};
            nexus_rt::new_resource!(Counter(u64));

            fn on_event(mut c: ResMut<Counter>, e: u64) {
                c.0 = c.0.wrapping_add(e);
            }

            let mut wb = WorldBuilder::new();
            wb.register(Counter(0));
            let mut world = wb.build();
            let mut h = on_event.into_handler(world.registry());

            for i in 0..BENCH_WARMUP as u64 {
                h.run(black_box(&mut world), black_box(i));
            }

            samples.clear();
            for i in 0..BENCH_SAMPLES as u64 {
                let s = rdtsc();
                h.run(black_box(&mut world), black_box(i));
                let e = rdtscp();
                samples.push(e.wrapping_sub(s));
            }
            black_box(world.resource::<Counter>().0);
            print_distribution("sync mono handler.run", &mut samples);
        }

        // -- Sync boxed handler (production mio path) --
        {
            use nexus_rt::{Handler, IntoHandler, ResMut, WorldBuilder};
            nexus_rt::new_resource!(Counter(u64));

            fn on_event(mut c: ResMut<Counter>, e: u64) {
                c.0 = c.0.wrapping_add(e);
            }

            let mut wb = WorldBuilder::new();
            wb.register(Counter(0));
            let mut world = wb.build();
            let mut h: Box<dyn Handler<u64>> =
                Box::new(on_event.into_handler(world.registry()));

            for i in 0..BENCH_WARMUP as u64 {
                h.run(black_box(&mut world), black_box(i));
            }

            samples.clear();
            for i in 0..BENCH_SAMPLES as u64 {
                let s = rdtsc();
                h.run(black_box(&mut world), black_box(i));
                let e = rdtscp();
                samples.push(e.wrapping_sub(s));
            }
            black_box(world.resource::<Counter>().0);
            print_distribution("sync Box<dyn Handler>", &mut samples);
        }

        println!();

        // -- Async: IO-woken task (pure poll overhead) --
        // Task returns Pending without self-waking. We manually re-queue
        // between polls to simulate the IO driver wake path.
        {
            struct IoTask(Rc<Cell<u64>>);
            impl Future for IoTask {
                type Output = ();
                fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                    self.0.set(self.0.get().wrapping_add(1));
                    Poll::Pending // no self-wake
                }
            }

            let counter = Rc::new(Cell::new(0u64));
            let mut executor = test_executor();
            let id = executor.spawn(IoTask(counter.clone()));
            let ptr = id.as_ptr();

            for _ in 0..BENCH_WARMUP {
                unsafe {
                    task::set_queued(ptr, true);
                    executor.incoming.push(ptr);
                }
                executor.poll();
            }
            counter.set(0);

            samples.clear();
            for _ in 0..BENCH_SAMPLES {
                unsafe {
                    task::set_queued(ptr, true);
                    executor.incoming.push(ptr);
                }
                let s = rdtsc();
                executor.poll();
                let e = rdtscp();
                samples.push(e.wrapping_sub(s));
            }
            black_box(counter.get());
            print_distribution("async poll (IO-woken, no self-wake)", &mut samples);
        }

        // -- Async: self-waking task (poll + wake overhead) --
        {
            struct BusyTask(Rc<Cell<u64>>);
            impl Future for BusyTask {
                type Output = ();
                fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                    self.0.set(self.0.get().wrapping_add(1));
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }

            let counter = Rc::new(Cell::new(0u64));
            let mut executor = test_executor();
            executor.spawn(BusyTask(counter.clone()));

            for _ in 0..BENCH_WARMUP {
                executor.poll();
            }
            counter.set(0);

            samples.clear();
            for _ in 0..BENCH_SAMPLES {
                let s = rdtsc();
                executor.poll();
                let e = rdtscp();
                samples.push(e.wrapping_sub(s));
            }
            black_box(counter.get());
            print_distribution("async poll (self-waking)", &mut samples);
        }

        println!();

        // -- Async: spawn + poll churn (alloc overhead, separate) --
        {
            let mut executor = test_executor();

            for _ in 0..BENCH_WARMUP {
                executor.spawn(async { black_box(42u64); });
                executor.poll();
            }

            samples.clear();
            for _ in 0..BENCH_SAMPLES {
                let s = rdtsc();
                executor.spawn(async { black_box(42u64); });
                executor.poll();
                let e = rdtscp();
                samples.push(e.wrapping_sub(s));
            }
            print_distribution("async spawn+poll (alloc churn)", &mut samples);
        }

        println!("\n--- Batched ({BATCH} iterations per sample, amortizes rdtsc overhead) ---\n");

        // -- Batched sync boxed (0 params — pure dispatch overhead) --
        {
            use nexus_rt::{Handler, IntoHandler, WorldBuilder};

            fn on_event(e: u64) {
                black_box(e);
            }

            let wb = WorldBuilder::new();
            let mut world = wb.build();
            let mut h: Box<dyn Handler<u64>> =
                Box::new(on_event.into_handler(world.registry()));

            for i in 0..BENCH_WARMUP as u64 {
                h.run(black_box(&mut world), black_box(i));
            }

            samples.clear();
            for batch in 0..BENCH_SAMPLES {
                let base = (batch * BATCH) as u64;
                let s = rdtsc();
                for j in 0..BATCH as u64 {
                    h.run(black_box(&mut world), black_box(base + j));
                }
                let e = rdtscp();
                samples.push(e.wrapping_sub(s) / BATCH as u64);
            }
            print_distribution("sync Box<dyn Handler> 0p (bat)", &mut samples);
        }

        // -- Batched sync boxed (1 param — with param resolution) --
        {
            use nexus_rt::{Handler, IntoHandler, ResMut, WorldBuilder};
            nexus_rt::new_resource!(Counter(u64));

            fn on_event(mut c: ResMut<Counter>, e: u64) {
                c.0 = c.0.wrapping_add(e);
            }

            let mut wb = WorldBuilder::new();
            wb.register(Counter(0));
            let mut world = wb.build();
            let mut h: Box<dyn Handler<u64>> =
                Box::new(on_event.into_handler(world.registry()));

            for i in 0..BENCH_WARMUP as u64 {
                h.run(black_box(&mut world), black_box(i));
            }

            samples.clear();
            for batch in 0..BENCH_SAMPLES {
                let base = (batch * BATCH) as u64;
                let s = rdtsc();
                for j in 0..BATCH as u64 {
                    h.run(black_box(&mut world), black_box(base + j));
                }
                let e = rdtscp();
                samples.push(e.wrapping_sub(s) / BATCH as u64);
            }
            black_box(world.resource::<Counter>().0);
            print_distribution("sync Box<dyn Handler> 1p (bat)", &mut samples);
        }

        // -- Batched async IO-woken --
        {
            struct IoTask(Rc<Cell<u64>>);
            impl Future for IoTask {
                type Output = ();
                fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                    self.0.set(self.0.get().wrapping_add(1));
                    Poll::Pending
                }
            }

            let counter = Rc::new(Cell::new(0u64));
            let mut executor = test_executor();
            let id = executor.spawn(IoTask(counter.clone()));
            let ptr = id.as_ptr();

            for _ in 0..BENCH_WARMUP {
                unsafe {
                    task::set_queued(ptr, true);
                    executor.incoming.push(ptr);
                }
                executor.poll();
            }
            counter.set(0);

            samples.clear();
            for _ in 0..BENCH_SAMPLES {
                // Pre-queue BATCH tasks (same task re-queued BATCH times)
                for _ in 0..BATCH {
                    unsafe {
                        task::set_queued(ptr, true);
                        executor.incoming.push(ptr);
                    }
                }
                let s = rdtsc();
                executor.poll();
                let e = rdtscp();
                samples.push(e.wrapping_sub(s) / BATCH as u64);
            }
            black_box(counter.get());
            print_distribution("async poll IO-woken (batched)", &mut samples);
        }

        // -- Batched async self-waking (N tasks) --
        {
            struct BusyTask(Rc<Cell<u64>>);
            impl Future for BusyTask {
                type Output = ();
                fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                    self.0.set(self.0.get().wrapping_add(1));
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }

            let counter = Rc::new(Cell::new(0u64));
            let mut executor = test_executor();
            executor.spawn(BusyTask(counter.clone()));

            for _ in 0..BENCH_WARMUP {
                executor.poll();
            }
            counter.set(0);

            // Each poll() does 1 task (self-waking goes to next cycle).
            // Batch by calling poll() BATCH times.
            samples.clear();
            for _ in 0..BENCH_SAMPLES {
                let s = rdtsc();
                for _ in 0..BATCH {
                    executor.poll();
                }
                let e = rdtscp();
                samples.push(e.wrapping_sub(s) / BATCH as u64);
            }
            black_box(counter.get());
            print_distribution("async poll self-wake (batched)", &mut samples);
        }

        println!();
        println!("Per-sample:  one rdtsc pair per iteration (includes ~20cy measurement floor)");
        println!("Batched:     one rdtsc pair per {BATCH} iterations (amortizes floor to ~0.2cy)");
    }
}
