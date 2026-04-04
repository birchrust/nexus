//! Single-threaded async runtime with pre-allocated task storage.
//!
//! [`Runtime`] owns an [`Executor`](crate::Executor) for spawned tasks, a
//! boxed root future, and an event-cycle timestamp. The root future is
//! driven to completion by [`block_on`](Runtime::block_on) or
//! [`block_on_busy`](Runtime::block_on_busy).
//!
//! Spawned tasks live in fixed-size slab slots (zero allocation after
//! init). The root future is boxed separately — it can be arbitrarily
//! large without competing for slab capacity.
//!
//! # Thread-local spawn
//!
//! [`spawn`](crate::spawn) is a free function that pushes a task into the
//! current runtime via a thread-local pointer set during `block_on`. This
//! mirrors `tokio::spawn` ergonomics. Calling it outside `block_on` panics.
//!
//! # Event timestamp
//!
//! A single [`Instant::now()`] is taken after each IO poll cycle (or at
//! loop entry when no IO driver is present). All dispatch within that
//! cycle shares the same timestamp — one clock read per cycle, not per
//! event. Access via [`RuntimeHandle::event_time`].

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use crate::{Executor, TaskId, WorldCtx, task::TASK_HEADER_SIZE};
use crate::io::{IoDriver, IoHandle};
use crate::timer::{TimerDriver, TimerHandle};

// =============================================================================
// Thread-local runtime context
// =============================================================================

thread_local! {
    /// Raw pointer to the active runtime's executor (as a trait object).
    /// Set on `block_on` entry, cleared on exit.
    static CURRENT: Cell<Option<*mut dyn SpawnErased>> = const { Cell::new(None) };
}

/// Type-erased spawn interface. Avoids propagating `SLOT_SIZE` through
/// the thread-local. The `Pin<Box<dyn Future>>` is stored in the slab
/// slot — `Box` is pointer-sized, so the slab slot holds the Box (not
/// the future inline). This is the cost of type erasure.
trait SpawnErased {
    fn spawn_erased(&mut self, future: Pin<Box<dyn Future<Output = ()>>>) -> TaskId;
}

impl<const S: usize> SpawnErased for Executor<S> {
    fn spawn_erased(&mut self, future: Pin<Box<dyn Future<Output = ()>>>) -> TaskId {
        self.spawn(future)
    }
}

/// Spawn a task into the current runtime via the thread-local context.
///
/// The future is boxed to cross the type-erasure boundary (the
/// thread-local doesn't know `SLOT_SIZE`). The `Box` itself is
/// pointer-sized and fits in any slab slot.
///
/// Must be called from within [`Runtime::block_on`] or
/// [`Runtime::block_on_busy`]. Panics otherwise.
///
/// # Panics
///
/// - If called outside a runtime context.
pub fn spawn<F>(future: F) -> TaskId
where
    F: Future<Output = ()> + 'static,
{
    CURRENT.with(|cell| {
        let ptr = cell
            .get()
            .expect("spawn() called outside of Runtime::block_on");
        // SAFETY: The pointer is valid for the duration of block_on.
        // Single-threaded — no concurrent access.
        let executor = unsafe { &mut *ptr };
        executor.spawn_erased(Box::pin(future))
    })
}

// =============================================================================
// RuntimeHandle — Copy handle for tasks
// =============================================================================

/// [`Copy`] handle for accessing runtime state from async tasks.
///
/// Provides [`WorldCtx`] access, IO registration, timer scheduling,
/// and the current event-cycle timestamp.
#[derive(Clone, Copy)]
pub struct RuntimeHandle {
    ctx: WorldCtx,
    event_time: *const Cell<Instant>,
    io: IoHandle,
    timers: TimerHandle,
}

impl RuntimeHandle {
    /// Access the [`World`](nexus_rt::World) synchronously.
    ///
    /// Delegates to [`WorldCtx::with_world`].
    pub fn with_world<R>(&self, f: impl FnOnce(&mut nexus_rt::World) -> R) -> R {
        self.ctx.with_world(f)
    }

    /// Access the [`World`](nexus_rt::World) with shared access.
    ///
    /// Delegates to [`WorldCtx::with_world_ref`].
    pub fn with_world_ref<R>(&self, f: impl FnOnce(&nexus_rt::World) -> R) -> R {
        self.ctx.with_world_ref(f)
    }

    /// Timestamp taken after the most recent IO poll cycle.
    ///
    /// All events dispatched within the same cycle share this timestamp.
    /// One clock read per cycle, not per event.
    pub fn event_time(&self) -> Instant {
        // SAFETY: Pointer is valid for the lifetime of the Runtime.
        // Single-threaded — no concurrent mutation during task poll.
        unsafe { &*self.event_time }.get()
    }

    /// Returns the IO handle for registering mio sources.
    pub fn io(&self) -> IoHandle {
        self.io
    }

    /// Returns the timer handle for scheduling deadlines.
    pub fn timers(&self) -> TimerHandle {
        self.timers
    }

    /// Convenience: sleep for `duration`. Returns a [`Sleep`](crate::Sleep) future.
    pub fn sleep(&self, duration: Duration) -> crate::Sleep {
        self.timers.sleep(duration)
    }

    /// Convenience: sleep until `deadline`. Returns a [`Sleep`](crate::Sleep) future.
    pub fn sleep_until(&self, deadline: Instant) -> crate::Sleep {
        self.timers.sleep_until(deadline)
    }
}

// =============================================================================
// Runtime
// =============================================================================

/// Single-threaded async runtime with pre-allocated task storage.
///
/// `SLOT_SIZE` is the maximum size in bytes for spawned task futures.
/// The root future passed to `block_on` is boxed separately and is not
/// constrained by this limit. Typical IO futures (socket read loop +
/// small state) are 128–256 bytes.
///
/// # Examples
///
/// ```ignore
/// use nexus_async_rt::{Runtime, DefaultRuntime, spawn};
/// use nexus_rt::WorldBuilder;
///
/// let mut world = WorldBuilder::new().build();
///
/// let mut rt = DefaultRuntime::new(&mut world, 64);
/// let handle = rt.handle();
///
/// let output = rt.block_on(async move {
///     spawn(async move {
///         // IO task — lives in slab
///         let data = read_socket().await;
///         handle.with_world(|world| process(world, data));
///     });
///
///     // Root future coordinates, then returns a value
///     wait_for_shutdown().await;
///     "done"
/// });
///
/// assert_eq!(output, "done");
/// ```
pub struct Runtime<const SLOT_SIZE: usize> {
    /// Spawned task storage. SLOT_SIZE includes task header overhead.
    executor: Executor<SLOT_SIZE>,

    /// IO driver (mio). Owns the Poll instance and token→waker map.
    io: IoDriver,

    /// Timer driver. Min-heap of deadlines → task wakers.
    timers: TimerDriver,

    /// World access handle.
    ctx: WorldCtx,

    /// Event-cycle timestamp. Updated after each IO poll cycle.
    /// `Cell` for interior mutability — tasks read it through a raw
    /// pointer while the runtime updates it.
    event_time: Cell<Instant>,
}

/// Runtime with 256-byte future capacity (+ 24 byte header = 280 byte slots).
pub type DefaultRuntime = Runtime<{ 256 + TASK_HEADER_SIZE }>;

impl<const SLOT_SIZE: usize> Runtime<SLOT_SIZE> {
    /// Create a runtime with pre-allocated capacity for `task_capacity`
    /// spawned tasks.
    ///
    /// The [`World`](nexus_rt::World) must outlive the runtime.
    ///
    /// # Panics
    ///
    /// Panics if the mio `Poll` instance cannot be created (OS error).
    pub fn new(world: &mut nexus_rt::World, task_capacity: usize) -> Self {
        Self {
            executor: Executor::with_capacity(task_capacity),
            io: IoDriver::new(1024, 64).expect("failed to create mio::Poll"),
            timers: TimerDriver::new(64),
            ctx: WorldCtx::new(world),
            event_time: Cell::new(Instant::now()),
        }
    }

    /// Get a [`Copy`] handle for use inside async tasks.
    ///
    /// The handle provides [`World`](nexus_rt::World) access, IO
    /// registration, and the current event-cycle timestamp.
    pub fn handle(&mut self) -> RuntimeHandle {
        RuntimeHandle {
            ctx: self.ctx,
            event_time: &raw const self.event_time,
            io: IoHandle::new(&mut self.io),
            timers: TimerHandle::new(&mut self.timers),
        }
    }

    /// Drive the root future to completion. CPU-friendly.
    ///
    /// Parks the thread when no work is available. Uses
    /// [`std::thread::park`] as a baseline — IO and timer drivers
    /// (when wired) will replace this with `epoll_wait` / timeout.
    ///
    /// Returns the root future's output value.
    pub fn block_on<F>(&mut self, future: F) -> F::Output
    where
        F: Future + 'static,
    {
        self.run_loop(future, ParkMode::Park)
    }

    /// Drive the root future to completion. Busy-wait.
    ///
    /// Never parks, never yields, never makes a blocking syscall when
    /// idle. Continuously polls the task queue and IO driver (with zero
    /// timeout). Minimum wake latency at the cost of 100% CPU on the
    /// pinned core.
    ///
    /// Returns the root future's output value.
    pub fn block_on_busy<F>(&mut self, future: F) -> F::Output
    where
        F: Future + 'static,
    {
        self.run_loop(future, ParkMode::Spin)
    }

    /// Number of live spawned tasks.
    pub fn task_count(&self) -> usize {
        self.executor.task_count()
    }

    // =========================================================================
    // Core event loop
    // =========================================================================

    fn run_loop<F>(&mut self, future: F, mode: ParkMode) -> F::Output
    where
        F: Future + 'static,
    {
        // Box the root future — not constrained by SLOT_SIZE.
        let mut root: Pin<Box<dyn Future<Output = F::Output>>> = Box::pin(future);

        // Woken flag: set by the root waker, checked by the loop.
        // Avoids writing to the eventfd on same-thread wakeups.
        let woken = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let root_waker = Waker::from(std::sync::Arc::new(RootWake {
            woken: std::sync::Arc::clone(&woken),
            mio_waker: self.io.mio_waker(),
        }));
        let mut root_cx = Context::from_waker(&root_waker);

        // Install thread-local context for spawn().
        let executor_ptr: *mut dyn SpawnErased = &mut self.executor;
        let _spawn_guard = RuntimeGuard::enter(executor_ptr);

        // Install TLS ready queue so timers and IO can wake tasks.
        let _ready_guard = crate::waker::set_ready_queue(self.executor.ready_queue_mut());

        // Take initial timestamp.
        self.event_time.set(Instant::now());

        loop {
            // 1. Poll root future if woken.
            if woken.swap(false, std::sync::atomic::Ordering::Acquire) {
                match root.as_mut().poll(&mut root_cx) {
                    Poll::Ready(output) => return output,
                    Poll::Pending => {}
                }
            }

            // 2. Poll spawned tasks.
            self.executor.poll();

            // 3. Fire expired timers → wakes tasks for this or next cycle.
            let now = Instant::now();
            self.timers.fire_expired(now);

            // 4. Poll mio for IO events.
            //    Timeout logic:
            //    - Spin mode: always ZERO (never block)
            //    - Park mode: if tasks are ready or root is woken, ZERO
            //      (just check for IO). Otherwise, block until the next
            //      timer deadline or until an IO event arrives.
            let has_work = self.executor.has_ready()
                || woken.load(std::sync::atomic::Ordering::Acquire);
            let mio_timeout = match mode {
                ParkMode::Park => {
                    if has_work {
                        Some(Duration::ZERO)
                    } else {
                        // Compute timeout from next timer deadline.
                        self.timers.next_deadline().map(|deadline| {
                            deadline.saturating_duration_since(Instant::now())
                        })
                        // None = no timers, block indefinitely until IO.
                    }
                }
                ParkMode::Spin => Some(Duration::ZERO),
            };
            let _ = self.io.poll_io(mio_timeout);

            // 5. Update event timestamp (after IO poll returns).
            self.event_time.set(Instant::now());
        }
    }
}

// =============================================================================
// Park mode
// =============================================================================

#[derive(Clone, Copy)]
enum ParkMode {
    /// Park the thread when idle. CPU-friendly.
    Park,
    /// Spin-poll. Never yield. Latency-optimal.
    Spin,
}

// =============================================================================
// Root future waker — woken flag + mio waker for epoll unpark
// =============================================================================

/// Root future waker. Sets an `AtomicBool` flag and only writes to the
/// mio eventfd if the flag wasn't already set (coalescing). The poll
/// loop checks and resets the flag each iteration.
///
/// Same-thread wakeups (the common case in single-threaded) set the
/// flag for free — no syscall. Cross-thread wakeups (rare) also poke
/// the mio waker to break `epoll_wait`.
struct RootWake {
    woken: std::sync::Arc<std::sync::atomic::AtomicBool>,
    mio_waker: std::sync::Arc<mio::Waker>,
}

impl Wake for RootWake {
    fn wake(self: std::sync::Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &std::sync::Arc<Self>) {
        let was_woken = self.woken.swap(true, std::sync::atomic::Ordering::Release);
        if !was_woken {
            // Flag was false → runtime might be parked in epoll_wait.
            // Poke the eventfd to unblock it.
            let _ = self.mio_waker.wake();
        }
    }
}

// =============================================================================
// RAII guard for thread-local runtime context
// =============================================================================

struct RuntimeGuard {
    prev: Option<*mut dyn SpawnErased>,
}

impl RuntimeGuard {
    fn enter(executor: *mut dyn SpawnErased) -> Self {
        let prev = CURRENT.with(|cell| cell.replace(Some(executor)));
        Self { prev }
    }
}

impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        CURRENT.with(|cell| cell.set(self.prev));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_rt::{Handler, IntoHandler, Res, ResMut, WorldBuilder};

    nexus_rt::new_resource!(Val(u64));
    nexus_rt::new_resource!(Out(u64));

    #[test]
    fn block_on_returns_value() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(42));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 4);
        let result = rt.block_on(async { 42u64 });
        assert_eq!(result, 42);
    }

    #[test]
    fn block_on_with_world_access() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(42));
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 4);
        let handle = rt.handle();

        let result = rt.block_on(async move {
            handle.with_world(|world| {
                let v = world.resource::<Val>().0;
                world.resource_mut::<Out>().0 = v + 10;
            });
            handle.with_world_ref(|world| world.resource::<Out>().0)
        });

        assert_eq!(result, 52);
    }

    #[test]
    fn block_on_with_pre_resolved_handler() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(42));
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 4);
        let handle = rt.handle();

        let mut h = (|val: Res<Val>, mut out: ResMut<Out>, event: u64| {
            out.0 = val.0 + event;
        })
        .into_handler(world.registry());

        let result = rt.block_on(async move {
            handle.with_world(|world| h.run(world, 10));
            handle.with_world_ref(|world| world.resource::<Out>().0)
        });

        assert_eq!(result, 52);
    }

    #[test]
    fn spawn_from_root_future() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 8);
        let handle = rt.handle();

        rt.block_on(async move {
            for i in 1..=3u64 {
                let h = handle;
                spawn(async move {
                    h.with_world(|world| {
                        world.resource_mut::<Out>().0 += i;
                    });
                });
            }

            // Yield once so spawned tasks get polled.
            YieldOnce(false).await;
        });

        assert_eq!(world.resource::<Out>().0, 6); // 1 + 2 + 3
    }

    #[test]
    fn block_on_busy_returns_value() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(7));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 4);
        let result = rt.block_on_busy(async { 6 * 7 });
        assert_eq!(result, 42);
    }

    #[test]
    fn block_on_busy_with_spawned_tasks() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 8);
        let handle = rt.handle();

        rt.block_on_busy(async move {
            let h = handle;
            spawn(async move {
                h.with_world(|world| {
                    world.resource_mut::<Out>().0 = 99;
                });
            });

            YieldOnce(false).await;
        });

        assert_eq!(world.resource::<Out>().0, 99);
    }

    #[test]
    fn event_time_is_set() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 4);
        let handle = rt.handle();

        let before = Instant::now();
        rt.block_on(async move {
            let t = handle.event_time();
            assert!(t >= before);
        });
    }

    #[test]
    #[should_panic(expected = "spawn() called outside of Runtime::block_on")]
    fn spawn_outside_runtime_panics() {
        spawn(async {});
    }

    // =========================================================================
    // Test helpers
    // =========================================================================

    /// Future that yields once (returns Pending), wakes itself, then
    /// completes on the next poll.
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

    #[test]
    fn sleep_completes() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 4);
        let handle = rt.handle();

        let before = Instant::now();
        rt.block_on(async move {
            handle.sleep(Duration::from_millis(50)).await;
        });
        let elapsed = before.elapsed();

        // Should have slept ~50ms (allow some tolerance).
        assert!(
            elapsed >= Duration::from_millis(40),
            "elapsed {elapsed:?} is too short"
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "elapsed {elapsed:?} is too long"
        );
    }

    #[test]
    fn sleep_in_spawned_task() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 8);
        let handle = rt.handle();

        let before = Instant::now();
        rt.block_on(async move {
            let h = handle;
            spawn(async move {
                h.sleep(Duration::from_millis(50)).await;
                h.with_world(|world| {
                    world.resource_mut::<Out>().0 = 42;
                });
            });

            // Wait for the spawned task to complete.
            // The root future sleeps a bit longer to give the task time.
            handle.sleep(Duration::from_millis(100)).await;
        });

        let elapsed = before.elapsed();
        assert!(elapsed >= Duration::from_millis(80));
        assert_eq!(world.resource::<Out>().0, 42);
    }
}
