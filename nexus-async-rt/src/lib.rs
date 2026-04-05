//! Single-threaded async runtime with pre-allocated task storage.
//!
//! Zero-allocation task spawn via nexus-slab byte slab (placement new).
//! Zero-allocation wakers (raw pointer as data, no Box, no Arc).
//! Designed for the hot path.
//!
//! ```ignore
//! use nexus_async_rt::*;
//!
//! let mut world = nexus_rt::WorldBuilder::new().build();
//!
//! // Choose your allocator
//! let alloc = DefaultUnboundedAlloc::new(64);
//!
//! // Build the runtime
//! let mut rt = Runtime::builder(&mut world, alloc)
//!     .tasks_per_cycle(128)
//!     .signal_handlers(true)
//!     .build();
//!
//! rt.block_on(async {
//!     // All runtime state via free functions:
//!     // ...
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

pub use alloc::{
    BoundedTaskAlloc, DefaultBoundedAlloc, DefaultUnboundedAlloc, TaskAlloc, UnboundedTaskAlloc,
};
pub use context::{event_time, io, shutdown_signal, sleep, sleep_until, with_world, with_world_ref};
pub use task::{TaskId, TASK_HEADER_SIZE};
pub use world_ctx::WorldCtx;
pub use io::IoHandle;
pub use shutdown::{ShutdownHandle, ShutdownSignal};
pub use net::{
    AsyncRead, AsyncWrite, OwnedReadHalf, OwnedWriteHalf, ReadHalf, TcpListener, TcpSocket,
    TcpStream, UdpSocket, WriteHalf,
};
pub use timer::{Sleep, TimerHandle};
pub use runtime::{DefaultRuntime, Runtime, RuntimeBuilder, spawn};

use std::future::Future;
use std::task::Poll;

use task::Task;
use waker::{set_poll_context, ReusableWaker};

/// Computes the internal slab slot size from user-visible future capacity.
///
/// The slab stores a task header (24 bytes) alongside each future.
/// Use this to compute the slot size for [`BoundedTaskAlloc`] or
/// [`UnboundedTaskAlloc`]:
///
/// ```ignore
/// use nexus_async_rt::{slot_size, BoundedTaskAlloc};
///
/// let alloc = BoundedTaskAlloc::<{ slot_size(256) }>::new(64);
/// ```
pub const fn slot_size(future_capacity: usize) -> usize {
    future_capacity + TASK_HEADER_SIZE
}

// =============================================================================
// Executor
// =============================================================================

/// Single-threaded async executor with pluggable task storage.
///
/// Generic over `A: TaskAlloc` — the allocator strategy (bounded or
/// unbounded byte slab). Use [`BoundedTaskAlloc`] for deterministic
/// behavior or [`UnboundedTaskAlloc`] for growable storage.
pub struct Executor<A: TaskAlloc> {
    /// Task allocator (bounded or unbounded byte slab).
    alloc: A,

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

impl<A: TaskAlloc> Executor<A> {
    /// Create an executor with the given allocator.
    pub fn new(alloc: A, initial_capacity: usize) -> Self {
        Self {
            alloc,
            incoming: Vec::with_capacity(initial_capacity),
            draining: Vec::with_capacity(initial_capacity),
            all_tasks: slab::Slab::with_capacity(initial_capacity),
            live_count: 0,
            tasks_per_cycle: DEFAULT_TASKS_PER_CYCLE,
            deferred_free: Vec::new(),
        }
    }

    /// Spawn an async task. Returns its [`TaskId`].
    pub fn spawn<F>(&mut self, future: F) -> TaskId
    where
        F: Future<Output = ()> + 'static,
    {
        let tracker_key = self.all_tasks.vacant_key();
        debug_assert!(
            u32::try_from(tracker_key).is_ok(),
            "more than 4 billion concurrent tasks — tracker_key overflow"
        );
        let task = Task::new(future, tracker_key as u32);

        let ptr = self.alloc.alloc_task(task);

        self.all_tasks.insert(ptr);

        unsafe { task::set_queued(ptr, true) };
        self.incoming.push(ptr);
        self.live_count += 1;

        TaskId(ptr)
    }

    /// Poll all ready tasks once.
    pub fn poll(&mut self) -> usize {
        let mut completed = 0;

        // Drain deferred frees from last cycle.
        for ptr in self.deferred_free.drain(..) {
            let key = unsafe { task::tracker_key(ptr) } as usize;
            unsafe { self.alloc.free(ptr) };
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

    /// Returns `true` if the allocator has space for at least one more
    /// task. Always true for unbounded allocators.
    ///
    /// Check this before calling [`spawn`](Self::spawn) with a bounded
    /// allocator to avoid a panic when the slab is full.
    pub fn has_capacity(&self) -> bool {
        self.alloc.max_capacity().is_none_or(|cap| self.live_count < cap)
    }

    /// Set the maximum tasks to poll per cycle.
    pub fn set_tasks_per_cycle(&mut self, limit: usize) {
        self.tasks_per_cycle = limit;
    }

    /// Returns the current poll limit.
    pub fn tasks_per_cycle(&self) -> usize {
        self.tasks_per_cycle
    }

    /// Complete a task: drop future, mark completed, release refcount.
    fn complete_task(&mut self, ptr: *mut u8) {
        unsafe { task::drop_task_future(ptr) };
        unsafe { task::set_completed(ptr) };
        self.live_count -= 1;

        let should_free = unsafe { task::ref_dec(ptr) };
        if should_free {
            let key = unsafe { task::tracker_key(ptr) } as usize;
            unsafe { self.alloc.free(ptr) };
            self.all_tasks.remove(key);
        }
    }

    /// Returns mutable references for TLS setup.
    pub(crate) fn poll_context_mut(&mut self) -> (&mut Vec<*mut u8>, &mut Vec<*mut u8>) {
        (&mut self.incoming, &mut self.deferred_free)
    }

    /// Run the executor until all tasks complete.
    ///
    /// Yields the thread when no tasks are ready. For the full
    /// runtime loop (with IO and timers), use [`Runtime::block_on`].
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
        self.incoming.retain(|p| *p != ptr);
        self.draining.retain(|p| *p != ptr);
        self.complete_task(ptr);
    }
}

impl<A: TaskAlloc> Drop for Executor<A> {
    fn drop(&mut self) {
        // Free deferred slots first (completed tasks whose last waker dropped).
        for ptr in self.deferred_free.drain(..) {
            unsafe { self.alloc.free(ptr) };
        }

        for (_, &ptr) in &self.all_tasks {
            // Drop the future if not already dropped.
            if !unsafe { task::is_completed(ptr) } {
                unsafe { task::drop_task_future(ptr) };
                unsafe { task::set_completed(ptr) };
                // Executor releases its reference.
                unsafe { task::ref_dec(ptr) };
            }

            // Check for outstanding waker clones.
            let rc = unsafe { task::ref_count(ptr) };
            debug_assert!(
                rc == 0,
                "executor dropped with {} outstanding waker clone(s) — \
                 all wakers must be dropped before the Runtime",
                rc,
            );

            unsafe { self.alloc.free(ptr) };
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

    fn test_executor() -> Executor<DefaultBoundedAlloc> {
        let alloc = DefaultBoundedAlloc::new(16);
        Executor::new(alloc, 16)
    }

    #[test]
    fn spawn_and_poll_immediate() {
        let mut executor = test_executor();

        let mut completed = false;
        let flag = &mut completed as *mut bool;

        executor.spawn(async move {
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
            executor.spawn(async move { c.set(c.get() + 1) });
        }

        assert_eq!(executor.task_count(), 3);
        assert_eq!(executor.poll(), 3);
        assert_eq!(counter.get(), 3);
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
        executor.poll();
        assert_eq!(executor.task_count(), 1);
        executor.cancel(id);
        assert_eq!(executor.task_count(), 0);
    }

    #[test]
    #[should_panic(expected = "exceeds byte slab slot size")]
    fn spawn_oversized_panics() {
        let big = [0u64; 128];
        let mut executor = test_executor();
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
            "small_task: {} bytes (+ {} header)",
            std::mem::size_of_val(&small_task()),
            TASK_HEADER_SIZE,
        );
        println!(
            "medium_task: {} bytes (+ {} header)",
            std::mem::size_of_val(&medium_task()),
            TASK_HEADER_SIZE,
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
                if self.0.get() >= 3 { return Poll::Ready(()); }
                cx.waker().wake_by_ref();
                cx.waker().wake_by_ref();
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }

        executor.spawn(WakeThrice(poll_count.clone()));
        assert_eq!(executor.poll(), 0);
        assert_eq!(poll_count.get(), 1);
        assert_eq!(executor.poll(), 0);
        assert_eq!(poll_count.get(), 2);
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
                if self.woken.get() { return Poll::Ready(()); }
                let cloned = cx.waker().clone();
                self.stored_waker = Some(cloned);
                self.woken.set(true);
                self.stored_waker.as_ref().unwrap().wake_by_ref();
                Poll::Pending
            }
        }

        executor.spawn(StoreAndWake { woken: woken.clone(), stored_waker: None });
        assert_eq!(executor.poll(), 0);
        assert!(woken.get());
        assert_eq!(executor.poll(), 1);
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
        impl Drop for DropTracker { fn drop(&mut self) { self.0.set(true); } }

        let d = dropped.clone();
        executor.spawn(async move { let _guard = DropTracker(d); });
        assert!(!dropped.get());
        executor.poll();
        assert!(dropped.get());
    }

    #[test]
    fn task_drop_on_cancel() {
        use std::cell::Cell;
        use std::rc::Rc;

        let dropped = Rc::new(Cell::new(false));
        let mut executor = test_executor();

        struct DropTracker(Rc<Cell<bool>>);
        impl Drop for DropTracker { fn drop(&mut self) { self.0.set(true); } }

        let d = dropped.clone();
        let id = executor.spawn(async move {
            let _guard = DropTracker(d);
            std::future::pending::<()>().await;
        });
        executor.poll();
        assert!(!dropped.get());
        executor.cancel(id);
        assert!(dropped.get());
    }

    #[test]
    fn cancel_frees_slot_for_reuse() {
        let mut executor = test_executor();
        for _ in 0..100 {
            let id = executor.spawn(async { std::future::pending::<()>().await; });
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
            impl Drop for DropCounter { fn drop(&mut self) { self.0.set(self.0.get() + 1); } }

            for _ in 0..3 {
                let guard = DropCounter(dropped.clone());
                executor.spawn(async move {
                    let _keep = guard;
                    std::future::pending::<()>().await;
                });
            }
            executor.poll();
            assert_eq!(dropped.get(), 0);
        }
        assert_eq!(dropped.get(), 3, "pending tasks not cleaned up on executor drop");

        let dropped2 = Rc::new(Cell::new(0u32));
        {
            let mut executor = test_executor();
            struct DropCounter2(Rc<Cell<u32>>);
            impl Drop for DropCounter2 { fn drop(&mut self) { self.0.set(self.0.get() + 1); } }

            for _ in 0..3 {
                let guard = DropCounter2(dropped2.clone());
                executor.spawn(async move { let _keep = guard; });
            }
        }
        assert_eq!(dropped2.get(), 3, "queued tasks not cleaned up on executor drop");
    }

    // =========================================================================
    // Drain snapshot
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
                if self.0.get() >= 10 { return Poll::Ready(()); }
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }

        executor.spawn(SelfWaker(poll_count.clone()));
        for expected in 1..10 {
            let completed = executor.poll();
            assert_eq!(poll_count.get(), expected);
            if expected < 10 { assert_eq!(completed, 0); }
        }
        assert_eq!(executor.poll(), 1);
        assert_eq!(poll_count.get(), 10);
    }

    // =========================================================================
    // Unbounded executor
    // =========================================================================

    #[test]
    fn unbounded_grows_beyond_initial() {
        let alloc = DefaultUnboundedAlloc::new(2);
        let mut executor = Executor::new(alloc, 2);
        for _ in 0..50 {
            executor.spawn(async {});
        }
        assert_eq!(executor.task_count(), 50);
        assert_eq!(executor.poll(), 50);
    }

    // =========================================================================
    // Poll limit
    // =========================================================================

    #[test]
    fn tasks_per_cycle_partial_drain() {
        use std::cell::Cell;
        use std::rc::Rc;

        let polled = Rc::new(Cell::new(0u32));
        let mut executor = test_executor();
        executor.set_tasks_per_cycle(3);

        for _ in 0..6 {
            let p = polled.clone();
            executor.spawn(async move { p.set(p.get() + 1); });
        }

        assert_eq!(executor.poll(), 3);
        assert_eq!(polled.get(), 3);
        assert_eq!(executor.poll(), 3);
        assert_eq!(polled.get(), 6);
    }

    #[test]
    fn tasks_per_cycle_unlimited() {
        let mut executor = test_executor();
        executor.set_tasks_per_cycle(usize::MAX);
        for _ in 0..16 {
            executor.spawn(async {});
        }
        assert_eq!(executor.poll(), 16);
    }

    // =========================================================================
    // Waker refcounting
    // =========================================================================

    #[test]
    fn waker_refcount_basic() {
        use std::cell::Cell;
        use std::rc::Rc;

        let dropped = Rc::new(Cell::new(false));
        let mut executor = test_executor();

        struct HoldsWaker {
            stored: Option<std::task::Waker>,
            dropped: Rc<Cell<bool>>,
        }
        impl Drop for HoldsWaker { fn drop(&mut self) { self.dropped.set(true); } }
        impl Future for HoldsWaker {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.stored.is_none() {
                    self.stored = Some(cx.waker().clone());
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }
        }

        let d = dropped.clone();
        let id = executor.spawn(HoldsWaker { stored: None, dropped: d });
        executor.poll();
        assert!(!dropped.get());
        executor.cancel(id);
        assert!(dropped.get());
    }

    #[test]
    fn waker_clone_and_wake_after_complete() {
        use std::cell::Cell;
        use std::rc::Rc;

        let counter = Rc::new(Cell::new(0u32));
        let external_waker: Rc<Cell<Option<std::task::Waker>>> = Rc::new(Cell::new(None));
        let ext = external_waker.clone();

        let mut executor = test_executor();

        struct StoreExternalWaker {
            ext: Rc<Cell<Option<std::task::Waker>>>,
            counter: Rc<Cell<u32>>,
            polled_once: bool,
        }
        impl Future for StoreExternalWaker {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                self.counter.set(self.counter.get() + 1);
                if !self.polled_once {
                    self.ext.set(Some(cx.waker().clone()));
                    self.polled_once = true;
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }
        }

        let c = counter.clone();
        executor.spawn(StoreExternalWaker { ext, counter: c, polled_once: false });
        executor.poll();
        assert_eq!(counter.get(), 1);

        let waker = external_waker.take().unwrap();
        let waker_bytes: &[*const (); 2] =
            unsafe { &*(&waker as *const std::task::Waker as *const [*const (); 2]) };
        let task_ptr = waker_bytes[1] as *mut u8;
        unsafe { task::set_queued(task_ptr, true) };
        executor.incoming.push(task_ptr);

        executor.poll();
        assert_eq!(counter.get(), 2);
        assert_eq!(executor.task_count(), 0);

        drop(waker);
        drop(executor);
    }

    #[test]
    fn waker_multiple_clones_all_drop() {
        use std::cell::Cell;
        use std::rc::Rc;

        let external_wakers: Rc<Cell<Vec<std::task::Waker>>> = Rc::new(Cell::new(Vec::new()));
        let ext = external_wakers.clone();

        let mut executor = test_executor();

        struct MultiClone {
            ext: Rc<Cell<Vec<std::task::Waker>>>,
            done: bool,
        }
        impl Future for MultiClone {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if !self.done {
                    let mut v = self.ext.take();
                    v.push(cx.waker().clone());
                    v.push(cx.waker().clone());
                    v.push(cx.waker().clone());
                    self.ext.set(v);
                    self.done = true;
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }
        }

        executor.spawn(MultiClone { ext, done: false });
        executor.poll();

        let wakers = external_wakers.take();
        let waker_bytes: &[*const (); 2] =
            unsafe { &*(&wakers[0] as *const std::task::Waker as *const [*const (); 2]) };
        let task_ptr = waker_bytes[1] as *mut u8;
        unsafe { task::set_queued(task_ptr, true) };
        executor.incoming.push(task_ptr);
        external_wakers.set(wakers);

        executor.poll();
        assert_eq!(executor.task_count(), 0);

        let wakers = external_wakers.take();
        drop(wakers);
        executor.poll();
    }

    #[test]
    fn cancel_with_stale_ready_entry() {
        let mut executor = test_executor();

        struct YieldOnce(bool);
        impl Future for YieldOnce {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.0 { Poll::Ready(()) } else {
                    self.0 = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }

        let id = executor.spawn(YieldOnce(false));
        executor.poll();
        executor.cancel(id);
        assert_eq!(executor.task_count(), 0);
        executor.poll();
        assert_eq!(executor.task_count(), 0);
    }

    #[test]
    fn wake_by_value_after_complete_decrements() {
        use std::cell::Cell;
        use std::rc::Rc;

        let external_waker: Rc<Cell<Option<std::task::Waker>>> = Rc::new(Cell::new(None));
        let ext = external_waker.clone();

        let mut executor = test_executor();

        struct StoreWaker {
            ext: Rc<Cell<Option<std::task::Waker>>>,
            first: bool,
        }
        impl Future for StoreWaker {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.first {
                    self.ext.set(Some(cx.waker().clone()));
                    self.first = false;
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }
        }

        executor.spawn(StoreWaker { ext, first: true });
        executor.poll();

        let waker = external_waker.take().unwrap();
        let waker_bytes: &[*const (); 2] =
            unsafe { &*(&waker as *const std::task::Waker as *const [*const (); 2]) };
        let task_ptr = waker_bytes[1] as *mut u8;
        unsafe { task::set_queued(task_ptr, true) };
        executor.incoming.push(task_ptr);

        executor.poll();
        assert_eq!(executor.task_count(), 0);

        drop(waker);
        executor.poll();
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
            "{name:<45} min:{min:>5}  p50:{p50:>5}  p90:{p90:>5}  p99:{p99:>5}  p999:{p999:>5}  p9999:{p9999:>5}  max:{max:>7}"
        );
    }

    const BENCH_WARMUP: usize = 10_000;
    const BENCH_SAMPLES: usize = 100_000;

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

        // -- Sync boxed handler --
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

        // -- Async: IO-woken task --
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

        // -- Async: self-waking task --
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

            for _ in 0..BENCH_WARMUP { executor.poll(); }
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

        // -- Async: spawn + poll churn --
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

        println!("\n--- Batched ({BATCH} iterations per sample) ---\n");

        // -- Batched sync boxed --
        {
            use nexus_rt::{Handler, IntoHandler, WorldBuilder};

            fn on_event(e: u64) { black_box(e); }

            let wb = WorldBuilder::new();
            let mut world = wb.build();
            let mut h: Box<dyn Handler<u64>> =
                Box::new(on_event.into_handler(world.registry()));

            for i in 0..BENCH_WARMUP as u64 { h.run(black_box(&mut world), black_box(i)); }

            samples.clear();
            for batch in 0..BENCH_SAMPLES {
                let base = (batch * BATCH) as u64;
                let s = rdtsc();
                for j in 0..BATCH as u64 { h.run(black_box(&mut world), black_box(base + j)); }
                let e = rdtscp();
                samples.push(e.wrapping_sub(s) / BATCH as u64);
            }
            print_distribution("sync Box<dyn Handler> 0p (bat)", &mut samples);
        }

        // -- Batched sync boxed 1p --
        {
            use nexus_rt::{Handler, IntoHandler, ResMut, WorldBuilder};
            nexus_rt::new_resource!(Counter(u64));

            fn on_event(mut c: ResMut<Counter>, e: u64) { c.0 = c.0.wrapping_add(e); }

            let mut wb = WorldBuilder::new();
            wb.register(Counter(0));
            let mut world = wb.build();
            let mut h: Box<dyn Handler<u64>> =
                Box::new(on_event.into_handler(world.registry()));

            for i in 0..BENCH_WARMUP as u64 { h.run(black_box(&mut world), black_box(i)); }

            samples.clear();
            for batch in 0..BENCH_SAMPLES {
                let base = (batch * BATCH) as u64;
                let s = rdtsc();
                for j in 0..BATCH as u64 { h.run(black_box(&mut world), black_box(base + j)); }
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
                unsafe { task::set_queued(ptr, true); executor.incoming.push(ptr); }
                executor.poll();
            }
            counter.set(0);

            samples.clear();
            for _ in 0..BENCH_SAMPLES {
                for _ in 0..BATCH {
                    unsafe { task::set_queued(ptr, true); executor.incoming.push(ptr); }
                }
                let s = rdtsc();
                executor.poll();
                let e = rdtscp();
                samples.push(e.wrapping_sub(s) / BATCH as u64);
            }
            black_box(counter.get());
            print_distribution("async poll IO-woken (batched)", &mut samples);
        }

        // -- Batched async self-wake --
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

            for _ in 0..BENCH_WARMUP { executor.poll(); }
            counter.set(0);

            samples.clear();
            for _ in 0..BENCH_SAMPLES {
                let s = rdtsc();
                for _ in 0..BATCH { executor.poll(); }
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
