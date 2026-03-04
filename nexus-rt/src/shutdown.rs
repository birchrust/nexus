//! Cooperative shutdown flag for event loops.
//!
//! [`Shutdown`] is a resource automatically registered by
//! [`WorldBuilder::build`](crate::WorldBuilder::build). Handlers trigger
//! shutdown via [`Res<Shutdown>`](crate::Res):
//!
//! ```
//! use nexus_rt::{Res, WorldBuilder};
//! use nexus_rt::shutdown::Shutdown;
//!
//! fn on_fatal(shutdown: Res<Shutdown>, _event: ()) {
//!     shutdown.shutdown();
//! }
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
//! With the `signals` feature, [`ShutdownHandle::enable_signals`] registers
//! SIGINT and SIGTERM handlers that flip the shutdown flag automatically.
//! Targets Linux infrastructure — not supported on Windows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cooperative shutdown flag — registered as a [`World`](crate::World) resource.
///
/// Interior-mutable via [`AtomicBool`], so it works through shared
/// [`Res<Shutdown>`](crate::Res) access. Uses [`Relaxed`](Ordering::Relaxed)
/// ordering — the flag is checked once per poll iteration, not on a
/// hot path requiring memory fencing.
pub struct Shutdown {
    flag: Arc<AtomicBool>,
}

impl Shutdown {
    pub(crate) fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns `true` if shutdown has been triggered.
    pub fn is_shutdown(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Trigger shutdown. Typically called from a handler via
    /// `Res<Shutdown>`.
    pub fn shutdown(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Create a [`ShutdownHandle`] sharing the same flag.
    pub(crate) fn handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            flag: Arc::clone(&self.flag),
        }
    }
}

/// External handle for the event loop to check shutdown status.
///
/// Shares the same [`AtomicBool`] as the [`Shutdown`] resource inside
/// [`World`](crate::World). Obtained via
/// [`World::shutdown_handle`](crate::World::shutdown_handle).
pub struct ShutdownHandle {
    flag: Arc<AtomicBool>,
}

impl ShutdownHandle {
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
    fn default_is_not_shutdown() {
        let s = Shutdown::new();
        assert!(!s.is_shutdown());
    }

    #[test]
    fn shutdown_flips_flag() {
        let s = Shutdown::new();
        s.shutdown();
        assert!(s.is_shutdown());
    }

    #[test]
    fn handle_sees_shutdown() {
        let s = Shutdown::new();
        let h = s.handle();

        assert!(!h.is_shutdown());
        s.shutdown();
        assert!(h.is_shutdown());
    }

    #[test]
    fn handle_can_trigger_shutdown() {
        let s = Shutdown::new();
        let h = s.handle();

        h.shutdown();
        assert!(s.is_shutdown());
    }

    #[test]
    fn handle_from_world() {
        let world = crate::WorldBuilder::new().build();
        let handle = world.shutdown_handle();

        assert!(!handle.is_shutdown());
        world.resource::<Shutdown>().shutdown();
        assert!(handle.is_shutdown());
    }
}
