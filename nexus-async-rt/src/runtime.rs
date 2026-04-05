//! Single-threaded async runtime.
//!
//! [`Runtime`] owns an [`Executor`](crate::Executor) for spawned tasks, a
//! boxed root future, and an event-cycle timestamp. The root future is
//! driven to completion by [`block_on`](Runtime::block_on) or
//! [`block_on_busy`](Runtime::block_on_busy).
//!
//! Two spawn strategies:
//! - **`spawn_boxed()`** — Box-allocated. Default. No setup needed.
//! - **`spawn_slab()`** — Slab-allocated. Zero-alloc hot path.
//!   Requires slab configured via [`RuntimeBuilder::slab`].
//!
//! # Thread-local spawn
//!
//! [`spawn`] and [`spawn_slab`] are free functions that push tasks into
//! the current runtime via thread-local pointers set during `block_on`.
//! Calling them outside `block_on` panics.

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use crate::{Executor, TaskId, WorldCtx};
use crate::io::IoDriver;
use crate::timer::TimerDriver;

// =============================================================================
// Thread-local spawn context
// =============================================================================

thread_local! {
    /// Raw pointer to the active runtime's executor.
    /// Set on `block_on` entry, cleared on exit.
    static CURRENT: Cell<*mut Executor> = const { Cell::new(std::ptr::null_mut()) };
}

/// Spawn a Box-allocated task into the current runtime.
///
/// The future is Box-allocated — no slab setup needed. For zero-alloc
/// spawning on the hot path, use [`spawn_slab`] with a configured slab.
///
/// Must be called from within [`Runtime::block_on`] or
/// [`Runtime::block_on_busy`]. Panics otherwise.
///
/// # Panics
///
/// - If called outside a runtime context.
pub fn spawn_boxed<F>(future: F) -> TaskId
where
    F: Future<Output = ()> + 'static,
{
    CURRENT.with(|cell| {
        let ptr = cell.get();
        assert!(!ptr.is_null(), "spawn_boxed() called outside of Runtime::block_on");
        // SAFETY: pointer valid for duration of block_on. Single-threaded.
        let executor = unsafe { &mut *ptr };
        executor.spawn_boxed(future)
    })
}

/// Spawn a slab-allocated task into the current runtime.
///
/// Zero allocation — the task is placed directly into a pre-allocated
/// slab slot via TLS. Requires a slab configured via
/// [`RuntimeBuilder::slab`].
///
/// The const generic `S` must match the slot size configured on the
/// runtime builder. Use [`slot_size`](crate::slot_size) to compute.
///
/// # Panics
///
/// - If called outside a runtime context.
/// - If no slab is configured.
pub fn spawn_slab<const S: usize, F>(future: F) -> TaskId
where
    F: Future<Output = ()> + 'static,
{
    CURRENT.with(|cell| {
        let ptr = cell.get();
        assert!(!ptr.is_null(), "spawn_slab() called outside of Runtime::block_on");
        let executor = unsafe { &mut *ptr };
        let tracker_key = executor.next_tracker_key();
        let task_ptr = crate::alloc::slab_spawn::<F, S>(future, tracker_key);
        executor.spawn_raw(task_ptr)
    })
}

// =============================================================================
// Runtime
// =============================================================================

/// Single-threaded async runtime.
///
/// # Examples
///
/// ```ignore
/// use nexus_async_rt::{Runtime, spawn, spawn_slab, slot_size};
/// use nexus_rt::WorldBuilder;
///
/// let mut world = WorldBuilder::new().build();
///
/// // Simple — Box-allocated tasks
/// let mut rt = Runtime::new(&mut world);
/// rt.block_on(async {
///     spawn_boxed(async { /* Box-allocated */ });
/// });
///
/// // With slab for hot-path tasks
/// let mut rt = Runtime::builder(&mut world)
///     .slab::<256>(64)
///     .build();
/// rt.block_on(async {
///     spawn_boxed(async { /* Box-allocated */ });
///     spawn_slab(async { /* slab-allocated */ });
/// });
/// ```
pub struct Runtime {
    /// Spawned task storage.
    executor: Executor,

    /// IO driver (mio).
    io: IoDriver,

    /// Timer driver.
    timers: TimerDriver,

    /// World access handle.
    ctx: WorldCtx,

    /// Event-cycle timestamp.
    event_time: Cell<Instant>,

    /// Graceful shutdown handle.
    shutdown: crate::ShutdownHandle,

    /// Optional slab allocator. Stored as a boxed trait object for
    /// type erasure (the const generic lives inside). The slab itself
    /// is accessed via TLS fn pointers — this field just owns the memory.
    _slab: Option<Box<dyn std::any::Any>>,

    /// RAII guard for slab TLS. Dropped after the run loop exits.
    _slab_guard: Option<crate::alloc::SlabGuard>,
}

impl Runtime {
    /// Create a runtime with default settings. Box-allocated tasks only.
    ///
    /// For slab allocation or custom configuration, use [`Runtime::builder`].
    pub fn new(world: &mut nexus_rt::World) -> Self {
        RuntimeBuilder::new(world).build()
    }

    /// Create a runtime via the builder pattern.
    pub fn builder(world: &mut nexus_rt::World) -> RuntimeBuilder<'_> {
        RuntimeBuilder::new(world)
    }

    /// Returns a [`ShutdownHandle`](crate::ShutdownHandle) for triggering or observing shutdown.
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

    /// Number of live spawned tasks.
    pub fn task_count(&self) -> usize {
        self.executor.task_count()
    }
}

// =============================================================================
// RuntimeBuilder
// =============================================================================

/// Type-erased closure that creates a slab + installs TLS.
type SlabInstaller = Box<dyn FnOnce() -> (Box<dyn std::any::Any>, crate::alloc::SlabGuard)>;

/// Builder for configuring a [`Runtime`].
///
/// # Examples
///
/// ```ignore
/// use nexus_async_rt::*;
///
/// let mut world = nexus_rt::WorldBuilder::new().build();
///
/// let mut rt = Runtime::builder(&mut world)
///     .tasks_per_cycle(128)
///     .slab::<256>(64)
///     .signal_handlers(true)
///     .build();
/// ```
pub struct RuntimeBuilder<'w> {
    world: &'w mut nexus_rt::World,
    tasks_per_cycle: usize,
    queue_capacity: usize,
    event_capacity: usize,
    token_capacity: usize,
    signal_handlers: bool,
    /// Type-erased slab + guard installer. None = no slab (Box-only).
    slab_installer: Option<SlabInstaller>,
}

impl<'w> RuntimeBuilder<'w> {
    fn new(world: &'w mut nexus_rt::World) -> Self {
        Self {
            world,
            tasks_per_cycle: crate::DEFAULT_TASKS_PER_CYCLE,
            queue_capacity: 64,
            event_capacity: 1024,
            token_capacity: 64,
            signal_handlers: false,
            slab_installer: None,
        }
    }

    /// Maximum tasks polled per cycle before yielding to check IO.
    /// Default: 64.
    pub fn tasks_per_cycle(mut self, limit: usize) -> Self {
        self.tasks_per_cycle = limit;
        self
    }

    /// Pre-allocated capacity for internal queues. Default: 64.
    pub fn queue_capacity(mut self, cap: usize) -> Self {
        self.queue_capacity = cap;
        self
    }

    /// Maximum IO events processed per epoll cycle. Default: 1024.
    pub fn event_capacity(mut self, cap: usize) -> Self {
        self.event_capacity = cap;
        self
    }

    /// Initial number of IO source slots. Default: 64.
    pub fn token_capacity(mut self, cap: usize) -> Self {
        self.token_capacity = cap;
        self
    }

    /// Install SIGTERM/SIGINT signal handlers. Default: false.
    pub fn signal_handlers(mut self, enable: bool) -> Self {
        self.signal_handlers = enable;
        self
    }

    /// Hand off a pre-created slab for [`spawn_slab`].
    ///
    /// `S` is the total slot size in bytes. The task header uses 32 bytes,
    /// so `Slab<256>` gives 224 bytes for the future. Most async IO
    /// futures are 128–256 bytes — `Slab<256>` or `Slab<512>` covers
    /// the common cases.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use nexus_slab::byte::unbounded::Slab;
    ///
    /// // SAFETY: single-threaded runtime.
    /// let slab = unsafe { Slab::<256>::with_chunk_capacity(64) };
    ///
    /// let mut rt = Runtime::builder(&mut world)
    ///     .slab(slab)
    ///     .build();
    /// ```
    pub fn slab<const S: usize>(
        mut self,
        slab: nexus_slab::byte::unbounded::Slab<S>,
    ) -> Self {
        const {
            assert!(S >= 64,
                "slab slot size must be at least 64 bytes (32 for task header + 32 for future)");
        }
        self.slab_installer = Some(Box::new(move || {
            let slab = Box::new(slab);
            let slab_ptr = std::ptr::from_ref(slab.as_ref()).cast::<u8>();
            let guard = crate::alloc::install_slab(
                slab_ptr,
                crate::alloc::slab_free_fn::<S>(),
            );
            (slab as Box<dyn std::any::Any>, guard)
        }));
        self
    }

    /// Build the runtime.
    pub fn build(self) -> Runtime {
        let io = IoDriver::new(self.event_capacity, self.token_capacity)
            .expect("failed to create mio::Poll");
        let mut shutdown = crate::ShutdownHandle::new();
        shutdown.set_mio_waker(io.mio_waker());

        let mut executor = Executor::new(self.queue_capacity);
        executor.set_tasks_per_cycle(self.tasks_per_cycle);

        let ctx = WorldCtx::new(self.world);
        let event_time = Cell::new(Instant::now());

        // Install slab if configured.
        let (slab, slab_guard) = self
            .slab_installer
            .map_or((None, None), |install| {
                let (slab, guard) = install();
                (Some(slab), Some(guard))
            });

        let rt = Runtime {
            executor,
            io,
            timers: TimerDriver::new(64),
            ctx,
            event_time,
            shutdown,
            _slab: slab,
            _slab_guard: slab_guard,
        };

        if self.signal_handlers {
            rt.install_signal_handlers();
        }

        rt
    }
}

// =============================================================================
// block_on / run_loop
// =============================================================================

impl Runtime {
    /// Drive the root future to completion. CPU-friendly.
    ///
    /// Parks the thread when no work is available.
    pub fn block_on<F>(&mut self, future: F) -> F::Output
    where
        F: Future + 'static,
    {
        self.run_loop(future, ParkMode::Park)
    }

    /// Drive the root future to completion. Busy-wait.
    ///
    /// Never parks. Minimum wake latency at 100% CPU.
    pub fn block_on_busy<F>(&mut self, future: F) -> F::Output
    where
        F: Future + 'static,
    {
        self.run_loop(future, ParkMode::Spin)
    }

    fn run_loop<F>(&mut self, future: F, mode: ParkMode) -> F::Output
    where
        F: Future + 'static,
    {
        // Install TLS context.
        let _ctx_guard = crate::context::install(
            self.ctx.as_ptr(),
            &raw mut self.io,
            &raw mut self.timers,
            &raw const self.event_time,
            std::sync::Arc::as_ptr(&self.shutdown.flag_ptr()),
        );

        let mut root: Pin<Box<dyn Future<Output = F::Output>>> = Box::pin(future);

        let woken = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let root_waker = Waker::from(std::sync::Arc::new(RootWake {
            woken: std::sync::Arc::clone(&woken),
            mio_waker: self.io.mio_waker(),
        }));
        let mut root_cx = Context::from_waker(&root_waker);

        // Install spawn TLS.
        let _spawn_guard = RuntimeGuard::enter(&raw mut self.executor);

        // Install waker TLS: ready queue + deferred free list.
        let (ready, deferred) = self.executor.poll_context_mut();
        let _ready_guard = crate::waker::set_poll_context(ready, deferred);

        self.event_time.set(Instant::now());

        loop {
            if woken.swap(false, std::sync::atomic::Ordering::Acquire)
                || self.shutdown.is_shutdown()
            {
                match root.as_mut().poll(&mut root_cx) {
                    Poll::Ready(output) => return output,
                    Poll::Pending => {}
                }
            }

            self.executor.poll();

            let now = Instant::now();
            self.timers.fire_expired(now);

            let has_work = self.executor.has_ready()
                || woken.load(std::sync::atomic::Ordering::Acquire);
            let mio_timeout = match mode {
                ParkMode::Park => {
                    if has_work {
                        Some(Duration::ZERO)
                    } else {
                        self.timers.next_deadline().map(|deadline| {
                            deadline.saturating_duration_since(Instant::now())
                        })
                    }
                }
                ParkMode::Spin => Some(Duration::ZERO),
            };
            if let Err(e) = self.io.poll_io(mio_timeout) {
                assert!(
                    e.kind() == std::io::ErrorKind::Interrupted,
                    "mio::Poll::poll failed: {e}"
                );
            }

            self.event_time.set(Instant::now());
        }
    }
}

// =============================================================================
// Park mode
// =============================================================================

#[derive(Clone, Copy)]
enum ParkMode {
    Park,
    Spin,
}

// =============================================================================
// Root future waker
// =============================================================================

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
            let _ = self.mio_waker.wake();
        }
    }
}

// =============================================================================
// RAII guard for spawn TLS
// =============================================================================

struct RuntimeGuard {
    prev: *mut Executor,
}

impl RuntimeGuard {
    fn enter(executor: *mut Executor) -> Self {
        let prev = CURRENT.with(|cell| cell.replace(executor));
        Self { prev }
    }
}

impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        CURRENT.with(|cell| cell.set(self.prev));
    }
}

// =============================================================================
// Tests
// =============================================================================

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

        let mut rt = Runtime::new(&mut world);
        let result = rt.block_on(async { 42u64 });
        assert_eq!(result, 42);
    }

    #[test]
    fn block_on_with_world_access() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(42));
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = Runtime::new(&mut world);

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

        let mut rt = Runtime::new(&mut world);

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

        let mut rt = Runtime::new(&mut world);

        rt.block_on(async move {
            for i in 1..=3u64 {
                spawn_boxed(async move {
                    crate::context::with_world(|world| {
                        world.resource_mut::<Out>().0 += i;
                    });
                });
            }

            YieldOnce(false).await;
        });

        assert_eq!(world.resource::<Out>().0, 6);
    }

    #[test]
    fn block_on_busy_returns_value() {
        let mut wb = WorldBuilder::new();
        wb.register(Val(7));
        let mut world = wb.build();

        let mut rt = Runtime::new(&mut world);
        let result = rt.block_on_busy(async { 6 * 7 });
        assert_eq!(result, 42);
    }

    #[test]
    fn block_on_busy_with_spawned_tasks() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = Runtime::new(&mut world);

        rt.block_on_busy(async move {
            spawn_boxed(async move {
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

        let mut rt = Runtime::new(&mut world);

        let before = Instant::now();
        rt.block_on(async move {
            let t = crate::context::event_time();
            assert!(t >= before);
        });
    }

    #[test]
    #[should_panic(expected = "spawn_boxed() called outside of Runtime::block_on")]
    fn spawn_outside_runtime_panics() {
        spawn_boxed(async {});
    }

    const SLOT: usize = 256;

    fn test_slab() -> nexus_slab::byte::unbounded::Slab<SLOT> {
        // SAFETY: single-threaded test.
        unsafe { nexus_slab::byte::unbounded::Slab::with_chunk_capacity(16) }
    }

    #[test]
    #[should_panic(expected = "spawn_slab() called without a slab")]
    fn spawn_slab_without_slab_panics() {
        let mut wb = WorldBuilder::new();
        let mut world = wb.build();
        let mut rt = Runtime::new(&mut world);

        rt.block_on(async {
            spawn_slab::<SLOT, _>(async {});
        });
    }

    #[test]
    fn spawn_slab_with_slab() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = Runtime::builder(&mut world)
            .slab(test_slab())
            .build();

        rt.block_on(async move {
            spawn_slab::<SLOT, _>(async move {
                crate::context::with_world(|world| {
                    world.resource_mut::<Out>().0 = 77;
                });
            });

            YieldOnce(false).await;
        });

        assert_eq!(world.resource::<Out>().0, 77);
    }

    #[test]
    fn mixed_spawn_and_spawn_slab() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = Runtime::builder(&mut world)
            .slab(test_slab())
            .build();

        rt.block_on(async move {
            // Box-allocated
            spawn_boxed(async move {
                crate::context::with_world(|world| {
                    world.resource_mut::<Out>().0 += 10;
                });
            });
            // Slab-allocated
            spawn_slab::<SLOT, _>(async move {
                crate::context::with_world(|world| {
                    world.resource_mut::<Out>().0 += 20;
                });
            });

            YieldOnce(false).await;
        });

        assert_eq!(world.resource::<Out>().0, 30);
    }

    // =========================================================================
    // Timer tests
    // =========================================================================

    #[test]
    fn sleep_completes() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = Runtime::new(&mut world);

        let before = Instant::now();
        rt.block_on(async move {
            crate::context::sleep(Duration::from_millis(50)).await;
        });
        let elapsed = before.elapsed();

        assert!(elapsed >= Duration::from_millis(40), "elapsed {elapsed:?} too short");
        assert!(elapsed < Duration::from_millis(200), "elapsed {elapsed:?} too long");
    }

    #[test]
    fn sleep_in_spawned_task() {
        let mut wb = WorldBuilder::new();
        wb.register(Out(0));
        let mut world = wb.build();

        let mut rt = Runtime::new(&mut world);

        let before = Instant::now();
        rt.block_on(async move {
            spawn_boxed(async move {
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
        let mut world = wb.build();
        let mut rt = Runtime::new(&mut world);

        let before = Instant::now();
        rt.block_on(async move {
            crate::context::sleep(Duration::ZERO).await;
        });
        assert!(before.elapsed() < Duration::from_millis(10));
    }

    #[test]
    fn sleep_past_deadline_ready_immediately() {
        let mut wb = WorldBuilder::new();
        let mut world = wb.build();
        let mut rt = Runtime::new(&mut world);

        let past = Instant::now() - Duration::from_secs(1);
        let before = Instant::now();
        rt.block_on(async move {
            crate::context::sleep_until(past).await;
        });
        assert!(before.elapsed() < Duration::from_millis(10));
    }

    // =========================================================================
    // Test helpers
    // =========================================================================

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
}
