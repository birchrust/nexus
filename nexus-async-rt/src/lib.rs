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

use std::collections::VecDeque;
use std::future::Future;
use std::task::{Context, Poll};

use task::Task;
use waker::{set_ready_queue, task_waker};

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

/// Builder for configuring an [`Executor`]'s allocation strategy.
///
/// `SLOT_SIZE` is the maximum size in bytes for spawned futures (not
/// including internal task header overhead).
///
/// # Examples
///
/// ```ignore
/// // Fixed capacity — panics on spawn if full
/// let executor = ExecutorBuilder::<256>::bounded(64);
///
/// // Growable — never fails, allocates new chunks as needed
/// let executor = ExecutorBuilder::<256>::unbounded(64);
/// ```
/// Computes internal slab slot size from user-visible future capacity.
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

    /// Task pointers ready to be polled. Dedup via `is_queued` flag.
    ready: VecDeque<*mut u8>,

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
            ready: VecDeque::with_capacity(capacity),
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
            ready: VecDeque::with_capacity(initial_chunk_capacity),
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

        // Mark as queued and push to ready queue.
        // SAFETY: ptr points to a live Task we just allocated.
        unsafe { task::set_queued(ptr, true) };
        self.ready.push_back(ptr);
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

        // Install TLS ready queue so wakers can push to it.
        // If the Runtime already installed a guard (covering timers + IO),
        // this creates a nested guard that restores correctly on drop.
        let _guard = set_ready_queue(&mut self.ready);

        // Drain the ready queue snapshot.
        let drain_count = self.ready.len();
        for _ in 0..drain_count {
            let Some(ptr) = self.ready.pop_front() else {
                break;
            };

            // Clear queued flag so the task can be re-queued by its waker.
            // SAFETY: ptr points to a live Task in the slab.
            unsafe { task::set_queued(ptr, false) };

            // Create a zero-alloc waker holding the task pointer.
            // ManuallyDrop elides the vtable drop call (our drop is a no-op).
            let waker = std::mem::ManuallyDrop::new(task_waker(ptr));
            let mut cx = Context::from_waker(&waker);

            // SAFETY: ptr points to a live Task. Future bytes are at
            // a fixed offset. Slab memory is stable (Pin sound).
            let poll_result = unsafe { task::poll_task(ptr, &mut cx) };

            match poll_result {
                Poll::Pending => {}
                Poll::Ready(()) => {
                    // Drop the future and free the slab slot.
                    // SAFETY: future is live, single drop.
                    unsafe { task::drop_task_future(ptr) };
                    // SAFETY: ptr was returned by alloc_task.
                    unsafe { self.slab.free_slot(ptr) };
                    self.live_count -= 1;
                    completed += 1;
                }
            }
        }

        completed
    }

    /// Number of live tasks.
    pub fn task_count(&self) -> usize {
        self.live_count
    }

    /// Returns `true` if any tasks are queued for polling.
    pub fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Returns a mutable reference to the ready queue.
    /// Used by Runtime to install TLS covering timers + IO.
    pub(crate) fn ready_queue_mut(&mut self) -> &mut VecDeque<*mut u8> {
        &mut self.ready
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
    pub fn cancel(&mut self, id: TaskId) {
        let ptr = id.0;
        // SAFETY: ptr points to a live Task.
        unsafe { task::drop_task_future(ptr) };
        unsafe { self.slab.free_slot(ptr) };
        self.live_count -= 1;

        // Remove from ready queue if queued.
        self.ready.retain(|p| *p != ptr);
    }
}

impl<const INTERNAL_SIZE: usize> Drop for Executor<INTERNAL_SIZE> {
    fn drop(&mut self) {
        // Drop all queued tasks.
        while let Some(ptr) = self.ready.pop_front() {
            unsafe { task::drop_task_future(ptr) };
            unsafe { self.slab.free_slot(ptr) };
        }
        // Note: tasks that are live but not queued (Pending, waiting for
        // external wakeup) are leaked. In practice, Runtime::block_on
        // runs to completion, and cancel() handles explicit cleanup.
        // A full solution would track all live task pointers.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;

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
}
