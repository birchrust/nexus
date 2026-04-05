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
//! event. Access via [`nexus_async_rt::event_time`].

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use crate::{Executor, TaskAlloc, TaskId, WorldCtx};
use crate::alloc::DefaultUnboundedAlloc;
use crate::io::IoDriver;
use crate::timer::TimerDriver;

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

impl<A: TaskAlloc> SpawnErased for Executor<A> {
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
///
/// let output = rt.block_on(async move {
///     spawn(async move {
///         // IO task — lives in slab
///         let data = read_socket().await;
///         crate::context::with_world(|world| process(world, data));
///     });
///
///     // Root future coordinates, then returns a value
///     wait_for_shutdown().await;
///     "done"
/// });
///
/// assert_eq!(output, "done");
/// ```
pub struct Runtime<A: TaskAlloc> {
    /// Spawned task storage.
    executor: Executor<A>,

    /// IO driver (mio). Owns the Poll instance and token→waker map.
    io: IoDriver,

    /// Timer driver.
    timers: TimerDriver,

    /// World access handle.
    ctx: WorldCtx,

    /// Event-cycle timestamp.
    event_time: Cell<Instant>,

    /// Graceful shutdown handle.
    shutdown: crate::ShutdownHandle,
}

/// Runtime with default unbounded allocator (256-byte future capacity).
pub type DefaultRuntime = Runtime<DefaultUnboundedAlloc>;

impl DefaultRuntime {
    /// Create a default runtime with unbounded task allocation.
    ///
    /// Convenience for the common case. For fine-grained control,
    /// use [`Runtime::builder`].
    pub fn new(world: &mut nexus_rt::World, queue_capacity: usize) -> Self {
        let alloc = DefaultUnboundedAlloc::new(queue_capacity);
        Runtime::builder(world, alloc).build()
    }
}

impl<A: TaskAlloc + 'static> Runtime<A> {
    /// Create a runtime via the builder pattern.
    pub fn builder(world: &mut nexus_rt::World, alloc: A) -> RuntimeBuilder<'_, A> {
        RuntimeBuilder::new(world, alloc)
    }

    /// Returns a [`ShutdownHandle`] for triggering or observing shutdown.
    pub fn shutdown_handle(&self) -> crate::ShutdownHandle {
        self.shutdown.clone()
    }

    /// Install signal handlers for SIGTERM and SIGINT.
    pub fn install_signal_handlers(&self) {
        crate::shutdown::install_signal_handlers(
            &self.shutdown.flag_ptr(),
            &self.io.mio_waker(),
        );
    }
}

// =============================================================================
// RuntimeBuilder
// =============================================================================

/// Builder for configuring a [`Runtime`].
///
/// # Examples
///
/// ```ignore
/// use nexus_async_rt::*;
///
/// let mut world = nexus_rt::WorldBuilder::new().build();
/// let alloc = DefaultUnboundedAlloc::new(64);
///
/// let mut rt = Runtime::builder(&mut world, alloc)
///     .tasks_per_cycle(128)
///     .signal_handlers(true)
///     .build();
/// ```
pub struct RuntimeBuilder<'w, A: TaskAlloc> {
    world: &'w mut nexus_rt::World,
    alloc: A,
    tasks_per_cycle: usize,
    queue_capacity: usize,
    event_capacity: usize,
    token_capacity: usize,
    signal_handlers: bool,
}

impl<'w, A: TaskAlloc + 'static> RuntimeBuilder<'w, A> {
    fn new(world: &'w mut nexus_rt::World, alloc: A) -> Self {
        Self {
            world,
            alloc,
            tasks_per_cycle: crate::DEFAULT_TASKS_PER_CYCLE,
            queue_capacity: 64,
            event_capacity: 1024,
            token_capacity: 64,
            signal_handlers: false,
        }
    }

    /// Maximum tasks polled per cycle before yielding to check IO.
    /// Default: 64.
    ///
    /// Lower values improve IO responsiveness (new data is noticed
    /// sooner). Higher values improve task throughput when many tasks
    /// are ready simultaneously.
    pub fn tasks_per_cycle(mut self, limit: usize) -> Self {
        self.tasks_per_cycle = limit;
        self
    }

    /// Pre-allocated capacity for internal queues. Default: 64.
    ///
    /// Sets the initial size of the ready queue and task tracking
    /// structures. Grows automatically if exceeded — this just avoids
    /// early reallocation.
    pub fn queue_capacity(mut self, cap: usize) -> Self {
        self.queue_capacity = cap;
        self
    }

    /// Maximum IO events processed per epoll cycle. Default: 1024.
    ///
    /// Size of the mio events buffer. If more events arrive than this
    /// in a single cycle, the excess is deferred to the next cycle.
    /// Increase for high-connection-count services.
    pub fn event_capacity(mut self, cap: usize) -> Self {
        self.event_capacity = cap;
        self
    }

    /// Initial number of IO source slots (sockets/fds). Default: 64.
    ///
    /// Each registered socket occupies one slot. Grows automatically
    /// if exceeded — this just avoids early reallocation.
    pub fn token_capacity(mut self, cap: usize) -> Self {
        self.token_capacity = cap;
        self
    }

    /// Install SIGTERM/SIGINT signal handlers for graceful shutdown.
    /// Default: false.
    ///
    /// When enabled, receiving SIGTERM or SIGINT sets the shutdown
    /// flag. Await [`ShutdownHandle::signal`] in the root future to
    /// observe it.
    pub fn signal_handlers(mut self, enable: bool) -> Self {
        self.signal_handlers = enable;
        self
    }

    /// Build the runtime.
    pub fn build(self) -> Runtime<A> {
        let io = IoDriver::new(self.event_capacity, self.token_capacity)
            .expect("failed to create mio::Poll");
        let mut shutdown = crate::ShutdownHandle::new();
        shutdown.set_mio_waker(io.mio_waker());

        let mut executor = Executor::new(self.alloc, self.queue_capacity);
        executor.set_tasks_per_cycle(self.tasks_per_cycle);

        let ctx = WorldCtx::new(self.world);
        let event_time = Cell::new(Instant::now());

        let rt = Runtime {
            executor,
            io,
            timers: TimerDriver::new(64),
            ctx,
            event_time,
            shutdown,
        };

        if self.signal_handlers {
            rt.install_signal_handlers();
        }

        rt
    }
}

impl<A: TaskAlloc + 'static> Runtime<A> {
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
        // Install TLS context — Runtime is at its final stack address.
        let _ctx_guard = crate::context::install(
            self.ctx.as_ptr(),
            &raw mut self.io,
            &raw mut self.timers,
            &raw const self.event_time,
            std::sync::Arc::as_ptr(&self.shutdown.flag_ptr()),
        );

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

        // Tell the IO driver about the root future's task pointer so
        // it can wake the root future via the woken flag instead of
        // pushing to the spawned-task ready queue.
        let root_task_ptr = {
            let waker_ptr = root_cx.waker() as *const Waker as *const [*const (); 2];
            // SAFETY: Waker layout validated by build script.
            unsafe { (*waker_ptr)[1] as *mut u8 }
        };
        // SAFETY: woken lives on this stack frame, outlives the run loop.
        // root_task_ptr is the value waker_to_ptr will extract from root_cx.
        unsafe {
            self.io.set_root_waker(
                root_task_ptr,
                std::sync::Arc::as_ptr(&woken),
            );
        }

        // Install thread-local context for spawn().
        let executor_ptr: *mut dyn SpawnErased = &mut self.executor;
        let _spawn_guard = RuntimeGuard::enter(executor_ptr);

        // Install TLS: ready queue + deferred free list for waker refcounting.
        let (ready, deferred) = self.executor.poll_context_mut();
        let _ready_guard = crate::waker::set_poll_context(ready, deferred);

        // Note: runtime context (world, io, timer, event_time, shutdown)
        // was already installed by RuntimeBuilder::build(). It persists
        // for the lifetime of the Runtime.

        // Take initial timestamp.
        self.event_time.set(Instant::now());

        loop {
            // 1. Poll root future if woken OR shutdown triggered.
            // Shutdown check ensures the root future sees the flag
            // even if its waker wasn't explicitly fired.
            if woken.swap(false, std::sync::atomic::Ordering::Acquire)
                || self.shutdown.is_shutdown()
            {
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
            if let Err(e) = self.io.poll_io(mio_timeout) {
                // Interrupt (EINTR) is expected — retry on next cycle.
                // Other errors indicate a serious OS-level problem.
                assert!(
                    e.kind() == std::io::ErrorKind::Interrupted,
                    "mio::Poll::poll failed: {e}"
                );
            }

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

        let result = rt.block_on(async move {
            crate::context::with_world(|world| {
                let v = world.resource::<Val>().0;
                world.resource_mut::<Out>().0 = v + 10;
            });
            crate::context::with_world_ref(|world| world.resource::<Out>().0)
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

        let mut h = (|val: Res<Val>, mut out: ResMut<Out>, event: u64| {
            out.0 = val.0 + event;
        })
        .into_handler(world.registry());

        let result = rt.block_on(async move {
            crate::context::with_world(|world| h.run(world, 10));
            crate::context::with_world_ref(|world| world.resource::<Out>().0)
        });

        assert_eq!(result, 52);
    }

    #[test]
    fn spawn_from_root_future() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 8);

        rt.block_on(async move {
            for i in 1..=3u64 {
                spawn(async move {
                    crate::context::with_world(|world| {
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

        rt.block_on_busy(async move {
            spawn(async move {
                crate::context::with_world(|world| {
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

        let before = Instant::now();
        rt.block_on(async move {
            let t = crate::context::event_time();
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

        let before = Instant::now();
        rt.block_on(async move {
            crate::context::sleep(Duration::from_millis(50)).await;
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

        let before = Instant::now();
        rt.block_on(async move {
            spawn(async move {
                crate::context::sleep(Duration::from_millis(50)).await;
                crate::context::with_world(|world| {
                    world.resource_mut::<Out>().0 = 42;
                });
            });

            crate::context::sleep(Duration::from_millis(100)).await;
        });

        let elapsed = before.elapsed();
        assert!(elapsed >= Duration::from_millis(80));
        assert_eq!(world.resource::<Out>().0, 42);
    }

    #[test]
    fn sleep_zero_duration_ready_immediately() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 4);

        let before = Instant::now();
        rt.block_on(async move {
            crate::context::sleep(Duration::ZERO).await;
        });
        let elapsed = before.elapsed();

        // Should complete almost instantly (< 10ms).
        assert!(
            elapsed < Duration::from_millis(10),
            "zero sleep took {elapsed:?}"
        );
    }

    #[test]
    fn sleep_past_deadline_ready_immediately() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 4);

        let past = Instant::now() - Duration::from_secs(1);
        let before = Instant::now();
        rt.block_on(async move {
            crate::context::sleep_until(past).await;
        });
        let elapsed = before.elapsed();

        assert!(
            elapsed < Duration::from_millis(10),
            "past deadline sleep took {elapsed:?}"
        );
    }

    #[test]
    fn multiple_timers_fire_in_order() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = DefaultRuntime::new(&mut world, 8);

        rt.block_on(async move {
            // Sleep 50ms then 50ms — total ~100ms.
            crate::context::sleep(Duration::from_millis(50)).await;
            crate::context::with_world(|world| {
                world.resource_mut::<Out>().0 = 1;
            });
            crate::context::sleep(Duration::from_millis(50)).await;
            crate::context::with_world(|world| {
                world.resource_mut::<Out>().0 = 2;
            });
        });

        assert_eq!(world.resource::<Out>().0, 2);
    }
}
