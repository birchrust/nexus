//! Single-threaded async runtime.
//!
//! Two spawn strategies:
//! - **`spawn_boxed()`** — Box-allocated. Default. No setup needed.
//! - **`spawn_slab()`** — Slab-allocated. Pre-allocated, zero-alloc
//!   hot path. Requires slab configured via [`RuntimeBuilder::slab`].
//!
//! ```ignore
//! use nexus_async_rt::*;
//! use nexus_slab::byte::unbounded::Slab;
//! use nexus_rt::WorldBuilder;
//!
//! let mut world = WorldBuilder::new().build();
//!
//! // Simple — Box-allocated tasks, no slab setup
//! let mut rt = Runtime::new(&mut world);
//! rt.block_on(async {
//!     spawn_boxed(async { /* Box-allocated */ });
//! });
//!
//! // Power user — with slab for hot-path tasks
//! // SAFETY: single-threaded runtime.
//! let slab = unsafe { Slab::<256>::with_chunk_capacity(64) };
//! let mut rt = Runtime::builder(&mut world)
//!     .slab_unbounded(slab)
//!     .build();
//! rt.block_on(async {
//!     spawn_boxed(async { /* Box-allocated, long-lived */ });
//!     spawn_slab(async { /* slab-allocated, hot path */ });
//! });
//! ```

// Single-threaded runtime — futures are intentionally !Send.
#![allow(clippy::future_not_send)]
#![cfg(unix)]

mod alloc;
mod context;
mod task;
mod waker;
mod world_ctx;
mod io;
pub mod net;
mod timer;
mod runtime;
mod shutdown;
mod backoff;
pub mod channel;
pub(crate) mod cross_wake;

// Re-export slab type for convenience — users create the slab and hand it to the builder.
pub use nexus_slab::byte::unbounded::Slab as ByteSlab;
pub use context::{after, after_delay, event_time, interval, interval_at, io, shutdown_signal, sleep, sleep_until, timeout, timeout_at, with_world, with_world_ref, yield_now};
pub use task::{TaskId, TASK_HEADER_SIZE};
pub use world_ctx::WorldCtx;
pub use io::IoHandle;
pub use shutdown::{ShutdownHandle, ShutdownSignal};
pub use net::{
    AsyncRead, AsyncWrite, OwnedReadHalf, OwnedWriteHalf, ReadHalf, TcpListener, TcpSocket,
    TcpStream, UdpSocket, WriteHalf,
};
pub use timer::{Elapsed, Interval, MissedTickBehavior, Sleep, Timeout, TimerHandle, YieldNow};
pub use backoff::{Backoff, BackoffBuilder, Exhausted};
pub use alloc::SlabClaim;
pub use runtime::{Runtime, RuntimeBuilder, spawn_boxed, spawn_slab, try_claim_slab, claim_slab};

use std::future::Future;
use std::task::Poll;

use task::Task;
use waker::{set_poll_context, ReusableWaker};

/// Minimum slab slot size: 64 bytes (32 for task header + 32 for future).
pub const MIN_SLOT_SIZE: usize = 64;

// =============================================================================
// Executor
// =============================================================================

/// Single-threaded async executor.
///
/// Manages task lifecycle: spawn, poll, complete, free. Tasks are
/// allocated via Box (default) or slab (via `spawn_slab`). Each
/// task's header contains a `free_fn` that knows how to deallocate
/// its own storage — the executor doesn't know or care which
/// allocator was used.
pub struct Executor {
    /// Incoming ready tasks. Wakers and spawn push here.
    /// Swapped with `draining` at the start of each poll cycle.
    incoming: Vec<*mut u8>,

    /// Tasks being drained this cycle. Iterated linearly.
    draining: Vec<*mut u8>,

    /// All live task pointers. Slab-indexed for O(1) removal.
    all_tasks: slab::Slab<*mut u8>,

    /// Number of live tasks.
    live_count: usize,

    /// Maximum tasks to poll per cycle before yielding to IO.
    tasks_per_cycle: usize,

    /// Completed task slots awaiting deferred free.
    deferred_free: Vec<*mut u8>,
}

/// Default poll limit.
const DEFAULT_TASKS_PER_CYCLE: usize = 64;

impl Executor {
    /// Create an executor.
    pub fn new(initial_capacity: usize) -> Self {
        Self {
            incoming: Vec::with_capacity(initial_capacity),
            draining: Vec::with_capacity(initial_capacity),
            all_tasks: slab::Slab::with_capacity(initial_capacity),
            live_count: 0,
            tasks_per_cycle: DEFAULT_TASKS_PER_CYCLE,
            deferred_free: Vec::new(),
        }
    }

    /// Reserve a tracker key for external allocation (slab spawn).
    pub(crate) fn next_tracker_key(&self) -> u32 {
        let key = self.all_tasks.vacant_key();
        debug_assert!(
            u32::try_from(key).is_ok(),
            "more than 4 billion concurrent tasks — tracker_key overflow"
        );
        key as u32
    }

    /// Spawn an async task via Box allocation. Returns its [`TaskId`].
    pub fn spawn_boxed<F>(&mut self, future: F) -> TaskId
    where
        F: Future<Output = ()> + 'static,
    {
        let tracker_key = self.all_tasks.vacant_key();
        debug_assert!(
            u32::try_from(tracker_key).is_ok(),
            "more than 4 billion concurrent tasks — tracker_key overflow"
        );
        let task = Task::new_boxed(future, tracker_key as u32);
        let ptr = Box::into_raw(Box::new(task)) as *mut u8;

        self.enqueue(ptr);
        TaskId(ptr)
    }

    /// Spawn a task with a pre-allocated pointer (from slab).
    ///
    /// The task at `ptr` must have been constructed with `Task::new_with_free`
    /// and a valid `free_fn` for the slab allocator.
    pub(crate) fn spawn_raw(&mut self, ptr: *mut u8) -> TaskId {
        self.enqueue(ptr);
        TaskId(ptr)
    }

    /// Common enqueue logic for spawn and spawn_raw.
    fn enqueue(&mut self, ptr: *mut u8) {
        self.all_tasks.insert(ptr);
        unsafe { task::set_queued(ptr, true) };
        self.incoming.push(ptr);
        self.live_count += 1;
    }

    /// Drain the cross-thread wake inbox into the local ready queue.
    ///
    /// Called at the start of each poll cycle. Tasks pushed from other
    /// threads via `CrossWakeQueue::push` are moved into `incoming`.
    /// Drains at most `limit` tasks (remaining are picked up next cycle).
    pub(crate) fn drain_cross_thread(
        &mut self,
        inbox: &mut crate::cross_wake::CrossWakeQueue,
        limit: usize,
    ) {
        let mut drained = 0;
        while drained < limit {
            match inbox.pop() {
                Some(task_ptr) => {
                    self.incoming.push(task_ptr);
                    drained += 1;
                }
                None => break,
            }
        }
    }

    /// Poll all ready tasks once.
    pub fn poll(&mut self) -> usize {
        let mut completed = 0;

        // Drain deferred frees from last cycle.
        for ptr in self.deferred_free.drain(..) {
            let key = unsafe { task::tracker_key(ptr) } as usize;
            // SAFETY: free_fn was set at spawn time.
            unsafe { task::free_task(ptr) };
            if self.all_tasks.contains(key) {
                self.all_tasks.remove(key);
            }
        }

        std::mem::swap(&mut self.incoming, &mut self.draining);

        let _guard = set_poll_context(&mut self.incoming, &mut self.deferred_free);

        let mut reusable = ReusableWaker::new();
        reusable.init();

        let limit = self.tasks_per_cycle.min(self.draining.len());
        let draining_ptr: *const Vec<*mut u8> = &raw const self.draining;
        let drain_slice = unsafe { &(&*draining_ptr)[..limit] };

        for &ptr in drain_slice {
            if unsafe { task::is_completed(ptr) } {
                continue;
            }

            unsafe { task::set_queued(ptr, false) };

            let cx = unsafe { reusable.set_task(ptr) };

            let poll_result = unsafe { task::poll_task(ptr, cx) };

            match poll_result {
                Poll::Pending => {}
                Poll::Ready(()) => {
                    self.complete_task(ptr);
                    completed += 1;
                }
            }
        }

        if limit < self.draining.len() {
            self.incoming.extend_from_slice(&self.draining[limit..]);
        }
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

    /// Set the maximum tasks to poll per cycle.
    pub fn set_tasks_per_cycle(&mut self, limit: usize) {
        self.tasks_per_cycle = limit;
    }

    /// Complete a task: drop future, mark completed, release refcount.
    fn complete_task(&mut self, ptr: *mut u8) {
        unsafe { task::drop_task_future(ptr) };
        unsafe { task::set_completed(ptr) };
        self.live_count -= 1;

        let should_free = unsafe { task::ref_dec(ptr) };
        if should_free {
            let key = unsafe { task::tracker_key(ptr) } as usize;
            unsafe { task::free_task(ptr) };
            self.all_tasks.remove(key);
        }
    }

    /// Returns mutable references for TLS setup.
    pub(crate) fn poll_context_mut(&mut self) -> (&mut Vec<*mut u8>, &mut Vec<*mut u8>) {
        (&mut self.incoming, &mut self.deferred_free)
    }

    /// Run the executor until all tasks complete.
    pub fn drain(&mut self) {
        while self.task_count() > 0 {
            if self.has_ready() {
                self.poll();
            } else {
                std::thread::yield_now();
            }
        }
    }

    /// Cancel a task by ID.
    pub fn cancel(&mut self, id: TaskId) {
        let ptr = id.0;
        // Skip if already completed (e.g. double-cancel or cancel after poll).
        if unsafe { task::is_completed(ptr) } {
            return;
        }
        self.incoming.retain(|p| *p != ptr);
        self.draining.retain(|p| *p != ptr);
        self.complete_task(ptr);
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Free deferred slots first (completed tasks whose last waker dropped).
        for ptr in self.deferred_free.drain(..) {
            unsafe { task::free_task(ptr) };
        }

        for (_, &ptr) in &self.all_tasks {
            // Drop the future if not already dropped.
            if !unsafe { task::is_completed(ptr) } {
                unsafe { task::drop_task_future(ptr) };
                unsafe { task::set_completed(ptr) };
                unsafe { task::ref_dec(ptr) };
            }

            let rc = unsafe { task::ref_count(ptr) };
            debug_assert!(
                rc == 0,
                "executor dropped with {} outstanding waker clone(s) — \
                 all wakers must be dropped before the Runtime",
                rc,
            );

            unsafe { task::free_task(ptr) };
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;
    use std::pin::Pin;
    use std::task::Context;

    fn test_executor() -> Executor {
        Executor::new(16)
    }

    // =========================================================================
    // Basic spawn + poll
    // =========================================================================

    #[test]
    fn spawn_and_poll_single_task() {
        let mut exec = test_executor();
        let mut done = false;
        let flag = &raw mut done;

        exec.spawn_boxed(async move {
            // SAFETY: single-threaded, flag lives on stack.
            unsafe { *flag = true };
        });

        assert_eq!(exec.task_count(), 1);
        let completed = exec.poll();
        assert_eq!(completed, 1);
        assert!(done);
        assert_eq!(exec.task_count(), 0);
    }

    #[test]
    fn spawn_multiple_tasks() {
        let mut exec = test_executor();

        for _ in 0..8 {
            exec.spawn_boxed(async {});
        }

        assert_eq!(exec.task_count(), 8);
        let completed = exec.poll();
        assert_eq!(completed, 8);
        assert_eq!(exec.task_count(), 0);
    }

    // =========================================================================
    // Pending tasks
    // =========================================================================

    #[test]
    fn pending_task_not_completed() {
        let mut exec = test_executor();

        // A future that is always pending.
        exec.spawn_boxed(std::future::pending::<()>());

        let completed = exec.poll();
        assert_eq!(completed, 0);
        assert_eq!(exec.task_count(), 1);
    }

    // =========================================================================
    // Waker: re-queue via wake_by_ref
    // =========================================================================

    #[test]
    fn immediate_task_completes() {
        let mut exec = test_executor();

        exec.spawn_boxed(async {
            // Immediately ready.
        });

        let completed = exec.poll();
        assert_eq!(completed, 1);
        assert_eq!(exec.task_count(), 0);
    }

    // =========================================================================
    // Self-waking task
    // =========================================================================

    #[test]
    fn self_waking_task_polled_again() {
        use std::cell::Cell;
        use std::rc::Rc;

        let mut exec = test_executor();

        let counter = Rc::new(Cell::new(0u32));
        let c = counter.clone();

        exec.spawn_boxed(async move {
            struct SelfWake {
                counter: Rc<Cell<u32>>,
            }
            impl Future for SelfWake {
                type Output = ();
                fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                    let n = self.counter.get();
                    self.counter.set(n + 1);
                    if n < 3 {
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                }
            }
            SelfWake { counter: c }.await;
        });

        // Drain all polls.
        let mut total = 0;
        for _ in 0..10 {
            total += exec.poll();
            if exec.task_count() == 0 {
                break;
            }
        }
        assert_eq!(total, 1); // completed once
        assert_eq!(counter.get(), 4); // polled 4 times
    }

    // =========================================================================
    // Cancel
    // =========================================================================

    #[test]
    fn cancel_task() {
        let mut exec = test_executor();
        let id = exec.spawn_boxed(std::future::pending::<()>());

        assert_eq!(exec.task_count(), 1);
        exec.cancel(id);
        assert_eq!(exec.task_count(), 0);
    }

    #[test]
    fn cancel_frees_slot_for_reuse() {
        let mut exec = test_executor();
        let id = exec.spawn_boxed(std::future::pending::<()>());
        exec.cancel(id);

        // Should be able to spawn again.
        exec.spawn_boxed(async {});
        assert_eq!(exec.task_count(), 1);
        exec.poll();
        assert_eq!(exec.task_count(), 0);
    }

    // =========================================================================
    // Poll limit (tasks_per_cycle)
    // =========================================================================

    #[test]
    fn poll_limit_respected() {
        let mut exec = test_executor();
        exec.set_tasks_per_cycle(2);

        for _ in 0..5 {
            exec.spawn_boxed(async {});
        }

        // Only 2 polled per cycle.
        let completed = exec.poll();
        assert_eq!(completed, 2);
        assert_eq!(exec.task_count(), 3);

        let completed = exec.poll();
        assert_eq!(completed, 2);
        assert_eq!(exec.task_count(), 1);

        let completed = exec.poll();
        assert_eq!(completed, 1);
        assert_eq!(exec.task_count(), 0);
    }

    // =========================================================================
    // Stale ready entries after cancel
    // =========================================================================

    #[test]
    fn cancel_with_stale_ready_entry() {
        use std::cell::Cell;
        use std::rc::Rc;

        let mut exec = test_executor();

        let polled = Rc::new(Cell::new(false));
        let p = polled.clone();

        // Spawn a self-waking task.
        struct WakeOnce(bool);
        impl Future for WakeOnce {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if !self.0 {
                    self.0 = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }
        }

        let id = exec.spawn_boxed(WakeOnce(false));

        // First poll: sets is_queued again via wake_by_ref.
        exec.poll();

        // Cancel while the task is in the ready queue.
        exec.cancel(id);

        // Spawn a new task to prove we don't crash on the stale pointer.
        exec.spawn_boxed(async move {
            p.set(true);
        });

        exec.poll();
        assert!(polled.get());
    }

    // =========================================================================
    // Refcount behavior
    // =========================================================================

    #[test]
    fn refcount_starts_at_one() {
        let task = Box::new(Task::new_boxed(async {}, 0));
        let ptr = Box::into_raw(task) as *mut u8;
        assert_eq!(unsafe { task::ref_count(ptr) }, 1);
        unsafe { task::free_task(ptr) };
    }

    #[test]
    fn executor_drop_cleans_up_queued_tasks() {
        let mut exec = test_executor();
        exec.spawn_boxed(std::future::pending::<()>());
        exec.spawn_boxed(std::future::pending::<()>());
        exec.poll(); // poll them once
        // Drop executor — should free all tasks without panic.
        drop(exec);
    }

    // =========================================================================
    // Dispatch latency (rough, not controlled)
    // =========================================================================

    #[test]
    #[ignore]
    fn dispatch_latency() {
        use std::time::Instant;

        struct Noop;
        impl Future for Noop {
            type Output = ();
            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }

        let mut exec = test_executor();
        exec.spawn_boxed(Noop);

        // Warmup.
        for _ in 0..10_000 {
            exec.poll();
        }

        let iters = 100_000;
        let start = Instant::now();
        for _ in 0..iters {
            exec.poll();
        }
        let elapsed = start.elapsed();
        let ns_per = elapsed.as_nanos() / iters;
        println!("dispatch: {ns_per} ns/poll (Box-allocated)");
        black_box(ns_per);
    }
}
