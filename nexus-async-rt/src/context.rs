//! Thread-local runtime context.
//!
//! All runtime state is accessible via free functions that read from
//! thread-local storage. The TLS slots are set by [`Runtime::block_on`]
//! and cleared on exit. All const-initialized for zero first-access cost.
//!
//! ```ignore
//! use nexus_async_rt::{spawn, with_world, sleep, io, shutdown_signal};
//!
//! rt.block_on(async {
//!     spawn(async {
//!         with_world(|world| { /* ... */ });
//!         sleep(Duration::from_secs(1)).await;
//!         let listener = TcpListener::bind(addr, io());
//!     });
//!     shutdown_signal().await;
//! });
//! ```

use std::cell::Cell;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::io::{IoDriver, IoHandle};
use crate::timer::{TimerDriver, TimerHandle};

// =============================================================================
// TLS slots — const-initialized, zero first-access cost
// =============================================================================

thread_local! {
    static CTX_WORLD: Cell<*mut nexus_rt::World> =
        const { Cell::new(std::ptr::null_mut()) };
    static CTX_IO: Cell<*mut IoDriver> =
        const { Cell::new(std::ptr::null_mut()) };
    static CTX_TIMER: Cell<*mut TimerDriver> =
        const { Cell::new(std::ptr::null_mut()) };
    static CTX_EVENT_TIME: Cell<*const Cell<Instant>> =
        const { Cell::new(std::ptr::null()) };
    static CTX_SHUTDOWN: Cell<*const AtomicBool> =
        const { Cell::new(std::ptr::null()) };
}

// =============================================================================
// Install / clear (called by Runtime::block_on)
// =============================================================================

/// Install runtime context into TLS. Called by RuntimeBuilder::build().
/// The context stays installed until the guard is dropped (Runtime::drop).
pub(crate) fn install(
    world: *mut nexus_rt::World,
    io: *mut IoDriver,
    timer: *mut TimerDriver,
    event_time: *const Cell<Instant>,
    shutdown_flag: *const AtomicBool,
) -> ContextGuard {
    let prev = PrevContext {
        world: CTX_WORLD.with(|c| c.replace(world)),
        io: CTX_IO.with(|c| c.replace(io)),
        timer: CTX_TIMER.with(|c| c.replace(timer)),
        event_time: CTX_EVENT_TIME.with(|c| c.replace(event_time)),
        shutdown: CTX_SHUTDOWN.with(|c| c.replace(shutdown_flag)),
    };
    ContextGuard { prev }
}

struct PrevContext {
    world: *mut nexus_rt::World,
    io: *mut IoDriver,
    timer: *mut TimerDriver,
    event_time: *const Cell<Instant>,
    shutdown: *const AtomicBool,
}

pub(crate) struct ContextGuard {
    prev: PrevContext,
}

impl Drop for ContextGuard {
    fn drop(&mut self) {
        CTX_WORLD.with(|c| c.set(self.prev.world));
        CTX_IO.with(|c| c.set(self.prev.io));
        CTX_TIMER.with(|c| c.set(self.prev.timer));
        CTX_EVENT_TIME.with(|c| c.set(self.prev.event_time));
        CTX_SHUTDOWN.with(|c| c.set(self.prev.shutdown));
    }
}

// =============================================================================
// Public free functions — the user-facing API
// =============================================================================

/// Access the [`World`](nexus_rt::World) with exclusive access.
///
/// Runs the closure synchronously inline. Must be called from within
/// [`Runtime::block_on`].
///
/// # Panics
///
/// Panics if called outside a runtime context.
pub fn with_world<R>(f: impl FnOnce(&mut nexus_rt::World) -> R) -> R {
    let ptr = CTX_WORLD.with(Cell::get);
    assert!(!ptr.is_null(), "with_world() called outside Runtime::block_on");
    // SAFETY: ptr set by install(), valid for Runtime lifetime.
    // Single-threaded — exclusive access.
    let world = unsafe { &mut *ptr };
    f(world)
}

/// Access the [`World`](nexus_rt::World) with shared access.
///
/// # Panics
///
/// Panics if called outside a runtime context.
pub fn with_world_ref<R>(f: impl FnOnce(&nexus_rt::World) -> R) -> R {
    let ptr = CTX_WORLD.with(Cell::get);
    assert!(!ptr.is_null(), "with_world_ref() called outside Runtime::block_on");
    let world = unsafe { &*ptr };
    f(world)
}

/// Returns the IO handle for registering mio sources.
///
/// # Panics
///
/// Panics if called outside a runtime context.
pub fn io() -> IoHandle {
    let ptr = CTX_IO.with(Cell::get);
    assert!(!ptr.is_null(), "io() called outside Runtime::block_on");
    // SAFETY: ptr valid for Runtime lifetime.
    IoHandle::new(unsafe { &mut *ptr })
}

/// Create a [`Sleep`](crate::Sleep) future that completes after `duration`.
///
/// # Panics
///
/// Panics if called outside a runtime context.
pub fn sleep(duration: Duration) -> crate::Sleep {
    let ptr = CTX_TIMER.with(Cell::get);
    assert!(!ptr.is_null(), "sleep() called outside Runtime::block_on");
    // SAFETY: ptr valid for Runtime lifetime.
    let handle = TimerHandle::new(unsafe { &mut *ptr });
    handle.sleep(duration)
}

/// Create a [`Sleep`](crate::Sleep) future that completes at `deadline`.
pub fn sleep_until(deadline: Instant) -> crate::Sleep {
    let ptr = CTX_TIMER.with(Cell::get);
    assert!(!ptr.is_null(), "sleep_until() called outside Runtime::block_on");
    let handle = TimerHandle::new(unsafe { &mut *ptr });
    handle.sleep_until(deadline)
}

/// Timestamp taken after the most recent IO poll cycle.
///
/// All events dispatched within the same cycle share this timestamp.
/// One clock read per cycle, not per event.
pub fn event_time() -> Instant {
    let ptr = CTX_EVENT_TIME.with(Cell::get);
    assert!(!ptr.is_null(), "event_time() called outside Runtime::block_on");
    // SAFETY: ptr valid for Runtime lifetime.
    unsafe { &*ptr }.get()
}

/// Returns a future that resolves when shutdown is triggered.
pub fn shutdown_signal() -> crate::ShutdownSignal {
    let ptr = CTX_SHUTDOWN.with(Cell::get);
    assert!(!ptr.is_null(), "shutdown_signal() called outside Runtime::block_on");
    // SAFETY: ptr points to AtomicBool inside Runtime's ShutdownHandle (Arc).
    // Reconstruct Arc temporarily, clone, forget original.
    let flag = unsafe { Arc::from_raw(ptr) };
    let cloned = Arc::clone(&flag);
    std::mem::forget(flag);
    crate::ShutdownSignal { flag: cloned }
}
