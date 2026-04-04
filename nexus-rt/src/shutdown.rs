//! Cooperative shutdown for event loops.
//!
//! [`Shutdown`] is a handler parameter that accesses the world's shutdown
//! flag directly — no resource registration needed. Handlers trigger
//! shutdown via [`Shutdown::trigger`]:
//!
//! ```
//! use nexus_rt::{WorldBuilder, IntoHandler, Handler};
//! use nexus_rt::shutdown::Shutdown;
//!
//! fn on_fatal(shutdown: Shutdown, _event: ()) {
//!     shutdown.trigger();
//! }
//!
//! let mut world = WorldBuilder::new().build();
//! let mut handler = on_fatal.into_handler(world.registry());
//! handler.run(&mut world, ());
//! assert!(world.shutdown_handle().is_shutdown());
//! ```
//!
//! The event loop owns a [`ShutdownHandle`] obtained from
//! [`World::shutdown_handle`](crate::World::shutdown_handle) and checks it
//! each iteration:
//!
//! ```
//! use nexus_rt::WorldBuilder;
//!
//! let mut world = WorldBuilder::new().build();
//! let shutdown = world.shutdown_handle();
//!
//! // typical event loop
//! while !shutdown.is_shutdown() {
//!     // poll drivers ...
//!     # break;
//! }
//! ```
//!
//! # Signal Support (Linux only)
//!
//! With the `signals` feature, `ShutdownHandle::enable_signals` registers
//! SIGINT and SIGTERM handlers that flip the shutdown flag automatically.
//! Targets Linux infrastructure — not supported on Windows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Handler parameter for cooperative shutdown.
///
/// Accesses the world's shutdown flag directly — not a resource.
/// Uses [`Relaxed`](Ordering::Relaxed) ordering — the flag is checked
/// once per poll iteration, not on a hot path requiring memory fencing.
/// Holds a reference to World's `AtomicBool` shutdown flag.
/// Lifetime-bound to the World borrow — cannot escape the dispatch frame.
pub struct Shutdown<'w>(pub(crate) &'w AtomicBool);

impl Shutdown<'_> {
    /// Returns `true` if shutdown has been triggered.
    #[inline(always)]
    pub fn is_shutdown(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Trigger shutdown. The event loop will exit after the current
    /// dispatch completes.
    pub fn trigger(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for Shutdown<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Shutdown")
            .field(&self.is_shutdown())
            .finish()
    }
}

/// External handle for the event loop to check shutdown status.
///
/// Shares the same [`AtomicBool`] as the world's shutdown flag.
/// Obtained via [`World::shutdown_handle`](crate::World::shutdown_handle).
pub struct ShutdownHandle {
    flag: Arc<AtomicBool>,
}

impl ShutdownHandle {
    pub(crate) fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }

    /// Returns `true` if shutdown has been triggered.
    pub fn is_shutdown(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Trigger shutdown from outside the event loop.
    pub fn shutdown(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Register SIGINT and SIGTERM handlers that trigger shutdown.
    ///
    /// Unix/Linux only. Uses [`signal_hook::flag::register`] — the signal
    /// handler simply flips the shared [`AtomicBool`]. Safe to call
    /// multiple times (subsequent calls are no-ops at the OS level for
    /// the same signal).
    ///
    /// # Errors
    ///
    /// Returns an error if the OS rejects the signal registration.
    ///
    /// # Platform Support
    ///
    /// Targets Linux infrastructure. Not supported on Windows.
    #[cfg(feature = "signals")]
    pub fn enable_signals(&self) -> std::io::Result<()> {
        signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&self.flag))?;
        signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&self.flag))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_not_shutdown_by_default() {
        let world = crate::WorldBuilder::new().build();
        let handle = world.shutdown_handle();
        assert!(!handle.is_shutdown());
    }

    #[test]
    fn shutdown_param_triggers() {
        let world = crate::WorldBuilder::new().build();
        let handle = world.shutdown_handle();
        let shutdown = Shutdown(world.shutdown_flag());

        assert!(!handle.is_shutdown());
        shutdown.trigger();
        assert!(handle.is_shutdown());
    }

    #[test]
    fn handle_can_trigger_shutdown() {
        let world = crate::WorldBuilder::new().build();
        let handle = world.shutdown_handle();
        assert!(!handle.is_shutdown());
        handle.shutdown();
        assert!(handle.is_shutdown());
    }

    #[test]
    fn shutdown_in_handler() {
        use crate::{Handler, IntoHandler};

        fn trigger_shutdown(shutdown: Shutdown, _event: ()) {
            shutdown.trigger();
        }

        let mut world = crate::WorldBuilder::new().build();
        let handle = world.shutdown_handle();

        let mut handler = trigger_shutdown.into_handler(world.registry());
        assert!(!handle.is_shutdown());
        handler.run(&mut world, ());
        assert!(handle.is_shutdown());
    }
}
