//! Graceful shutdown support.
//!
//! [`ShutdownSignal`] is a future that resolves when a shutdown is
//! requested — either by a Unix signal (SIGTERM, SIGINT) or by
//! explicitly calling [`ShutdownHandle::trigger`].
//!
//! The Runtime checks the shutdown flag each poll cycle. When set,
//! the root future can observe it via the `ShutdownSignal` future
//! and begin connection draining.
//!
//! # Usage
//!
//! ```ignore
//! let mut rt = Runtime::new(&mut world);
//!
//! // Install signal handlers (call once at startup).
//! rt.install_signal_handlers();
//!
//! rt.block_on(async move {
//!     spawn(connection_tasks...);
//!
//!     // Wait for SIGTERM/SIGINT.
//!     nexus_async_rt::shutdown_signal().await;
//!
//!     // Drain connections, flush buffers, etc.
//! });
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

/// Shared shutdown flag.
#[derive(Clone)]
pub struct ShutdownHandle {
    flag: Arc<AtomicBool>,
    /// Mio waker to break epoll_wait when shutdown is triggered.
    mio_waker: Option<Arc<mio::Waker>>,
}

impl ShutdownHandle {
    pub(crate) fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            mio_waker: None,
        }
    }

    /// Set the mio waker. Called by Runtime during construction.
    pub(crate) fn set_mio_waker(&mut self, waker: Arc<mio::Waker>) {
        self.mio_waker = Some(waker);
    }

    /// Trigger shutdown programmatically.
    ///
    /// Sets the flag and breaks epoll_wait so the runtime loop
    /// re-polls the root future.
    pub fn trigger(&self) {
        self.flag.store(true, Ordering::Release);
        if let Some(w) = &self.mio_waker {
            let _ = w.wake();
        }
    }

    /// Check if shutdown has been requested.
    pub fn is_shutdown(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// Get the underlying flag Arc for signal handler registration.
    pub(crate) fn flag_ptr(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }

    /// Returns a future that completes when shutdown is triggered.
    pub fn signal(&self) -> ShutdownSignal {
        ShutdownSignal {
            flag: Arc::as_ptr(&self.flag),
        }
    }
}

/// Future that resolves when shutdown is triggered.
///
/// Checked by the Runtime's poll loop — when the shutdown flag is set,
/// the root future gets re-polled automatically.
/// Future that resolves when shutdown is triggered.
///
/// Holds a raw pointer to the AtomicBool flag, valid for the lifetime
/// of the Runtime (which outlives `block_on` which outlives all tasks).
pub struct ShutdownSignal {
    pub(crate) flag: *const AtomicBool,
}

impl Future for ShutdownSignal {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        // SAFETY: flag points to the AtomicBool inside the Runtime's
        // ShutdownHandle (Arc-allocated, stable address). Valid for
        // Runtime lifetime.
        if unsafe { &*self.flag }.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        Poll::Pending
    }
}

/// Install signal handlers for SIGTERM and SIGINT that trigger shutdown.
///
/// Uses `signal-hook` for safe, portable signal registration. The
/// handler atomically sets the flag. The mio waker breaks epoll_wait
/// so the runtime notices the flag promptly.
pub fn install_signal_handlers(flag: &Arc<AtomicBool>, mio_waker: &Arc<mio::Waker>) {
    let waker_ref = Arc::clone(mio_waker);

    // signal-hook provides safe registration with proper cleanup.
    // The closure runs in signal context — only async-signal-safe
    // operations (atomic store + eventfd write).
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(flag))
        .expect("failed to register SIGTERM handler");
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(flag))
        .expect("failed to register SIGINT handler");

    // signal-hook::flag::register sets the AtomicBool on signal, but
    // we also need to break epoll_wait. Register a second handler that
    // fires the mio waker.
    unsafe {
        signal_hook::low_level::register(signal_hook::consts::SIGTERM, move || {
            let _ = waker_ref.wake();
        })
        .expect("failed to register SIGTERM waker");
    }
    let waker_ref2 = Arc::clone(mio_waker);
    unsafe {
        signal_hook::low_level::register(signal_hook::consts::SIGINT, move || {
            let _ = waker_ref2.wake();
        })
        .expect("failed to register SIGINT waker");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_handle_trigger() {
        let handle = ShutdownHandle::new();
        assert!(!handle.is_shutdown());
        handle.trigger();
        assert!(handle.is_shutdown());
    }

    #[test]
    fn shutdown_signal_resolves_after_trigger() {
        use crate::{Runtime, spawn};
        use nexus_rt::WorldBuilder;
        use std::cell::Cell;
        use std::rc::Rc;

        let wb = WorldBuilder::new();
        let mut world = wb.build();
        let mut rt = Runtime::new(&mut world);
        let shutdown = rt.shutdown_handle();

        let done = Rc::new(Cell::new(false));
        let flag = done.clone();

        // Trigger shutdown from a spawned task after a short delay.
        let sh = shutdown.clone();
        rt.block_on(async move {
            spawn(async move {
                crate::context::sleep(std::time::Duration::from_millis(50)).await;
                sh.trigger();
            });

            // Root future waits for shutdown.
            shutdown.signal().await;
            flag.set(true);
        });

        assert!(done.get());
    }
}
