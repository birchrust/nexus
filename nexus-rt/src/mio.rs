//! Mio IO driver for nexus-rt.
//!
//! Integrates [`mio`] as a driver following the
//! [`Installer`]/[`Plugin`](crate::Plugin) pattern. Handlers receive
//! [`mio::event::Event`] directly — no wrapper types.
//!
//! # Architecture
//!
//! - [`MioInstaller`] is the installer — consumed at setup, registers the
//!   [`MioDriver`] into [`WorldBuilder`] and returns a [`MioPoller`].
//! - [`MioPoller`] is the poll-time handle. `poll(world, timeout)` polls
//!   mio for readiness events and fires handlers.
//! - [`MioDriver`] is the World resource wrapping `mio::Poll` +
//!   `slab::Slab<S>`. Users register mio sources via `registry()` and
//!   manage handlers via `insert()`/`remove()`.
//!
//! # Handler lifecycle (move-out-fire)
//!
//! 1. User calls `driver.insert(handler)` → [`MioToken`]
//! 2. User calls `driver.registry().register(&mut source, token.into(), interest)`
//! 3. On readiness: poller removes handler from slab, fires it
//! 4. Handler re-registers itself if it wants more events
//!
//! Handlers that don't re-insert themselves are dropped after firing.
//! Stale tokens (already removed) are silently skipped.
//!
//! # Source lifecycle
//!
//! Mio sources (sockets, pipes, etc.) are registered with `mio::Registry`
//! **separately** from handlers. The driver does not track or own sources.
//!
//! When a handler is removed — either by [`MioDriver::remove`] or by the
//! move-out-fire pattern during [`MioPoller::poll`] — the mio source
//! remains registered with the kernel. The user is responsible for
//! calling `registry().deregister(&mut source)` if no replacement handler
//! will be inserted. Forgetting to deregister is safe (stale tokens are
//! skipped) but wastes kernel resources.

use std::io;
use std::marker::PhantomData;
use std::ops::DerefMut;
use std::time::Duration;

use crate::driver::Installer;
use crate::system::Handler;
use crate::world::{ResourceId, World, WorldBuilder};

/// Default mio events buffer capacity.
const DEFAULT_EVENT_CAPACITY: usize = 1024;

/// Default handler slab pre-allocation.
const DEFAULT_HANDLER_CAPACITY: usize = 64;

/// Newtype around a slab key, used as a mio token.
///
/// Obtained from [`MioDriver::insert`]. Convert to [`mio::Token`] via
/// `Into` for use with `mio::Registry::register`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MioToken(pub usize);

impl From<MioToken> for ::mio::Token {
    fn from(t: MioToken) -> ::mio::Token {
        ::mio::Token(t.0)
    }
}

impl From<::mio::Token> for MioToken {
    fn from(t: ::mio::Token) -> MioToken {
        MioToken(t.0)
    }
}

/// Configuration trait for generic IO driver code.
///
/// ZST annotation type that bundles the handler storage type with a
/// wrapping function. Library code parameterized over `C: MioConfig`
/// can insert and wrap handlers without knowing the concrete storage
/// strategy.
pub trait MioConfig: Send + 'static {
    /// The handler storage type (e.g. `Box<dyn Handler<mio::event::Event>>`).
    type Storage: DerefMut<Target = dyn Handler<::mio::event::Event>> + Send + 'static;

    /// Wrap a concrete handler into the storage type.
    fn wrap(handler: impl Handler<::mio::event::Event> + 'static) -> Self::Storage;
}

/// Boxed mio configuration — heap-allocates each handler.
pub struct BoxedMio;

impl MioConfig for BoxedMio {
    type Storage = Box<dyn Handler<::mio::event::Event>>;

    fn wrap(handler: impl Handler<::mio::event::Event> + 'static) -> Self::Storage {
        Box::new(handler)
    }
}

/// Inline mio configuration — stores handlers in a fixed-size buffer.
///
/// Panics if a handler exceeds the buffer size (256 bytes).
#[cfg(feature = "smartptr")]
pub struct InlineMio;

#[cfg(feature = "smartptr")]
impl MioConfig for InlineMio {
    type Storage = crate::FlatVirtual<::mio::event::Event, nexus_smartptr::B256>;

    fn wrap(handler: impl Handler<::mio::event::Event> + 'static) -> Self::Storage {
        let ptr: *const dyn Handler<::mio::event::Event> = &handler;
        // SAFETY: ptr's metadata (vtable) corresponds to handler's concrete type.
        unsafe { nexus_smartptr::Flat::new_raw(handler, ptr) }
    }
}

/// Flex mio configuration — inline with heap fallback.
///
/// Stores inline if the handler fits in 256 bytes, otherwise
/// heap-allocates. No panics.
#[cfg(feature = "smartptr")]
pub struct FlexMio;

#[cfg(feature = "smartptr")]
impl MioConfig for FlexMio {
    type Storage = crate::FlexVirtual<::mio::event::Event, nexus_smartptr::B256>;

    fn wrap(handler: impl Handler<::mio::event::Event> + 'static) -> Self::Storage {
        let ptr: *const dyn Handler<::mio::event::Event> = &handler;
        // SAFETY: ptr's metadata (vtable) corresponds to handler's concrete type.
        unsafe { nexus_smartptr::Flex::new_raw(handler, ptr) }
    }
}

/// World resource wrapping `mio::Poll` and a handler slab.
///
/// `S` is the handler storage type. Defaults to
/// `Box<dyn Handler<mio::event::Event>>`.
///
/// Users interact with this through `Res<MioDriver<S>>` (for
/// `registry()`) or `ResMut<MioDriver<S>>` (for `insert`/`remove`).
pub struct MioDriver<S = Box<dyn Handler<::mio::event::Event>>> {
    poll: ::mio::Poll,
    handlers: ::slab::Slab<S>,
}

impl<S> MioDriver<S> {
    /// Access the mio registry for registering/reregistering sources.
    ///
    /// `Poll::registry()` takes `&self`, so this works through
    /// `Res<MioDriver<S>>` (shared access).
    pub fn registry(&self) -> &::mio::Registry {
        self.poll.registry()
    }

    /// Insert a handler and return its token.
    ///
    /// The token maps to a `mio::Token` for use with
    /// `Registry::register`. Requires `ResMut<MioDriver<S>>`.
    pub fn insert(&mut self, handler: S) -> MioToken {
        MioToken(self.handlers.insert(handler))
    }

    /// Remove a handler by token.
    ///
    /// The caller is responsible for deregistering the mio source via
    /// `registry().deregister(&mut source)` if no replacement handler
    /// will be inserted. Failing to deregister is safe (stale events
    /// are skipped) but wastes kernel resources.
    ///
    /// # Panics
    ///
    /// Panics if the token is not present in the slab.
    pub fn remove(&mut self, token: MioToken) -> S {
        self.handlers.remove(token.0)
    }

    /// Returns `true` if the token has a handler in the slab.
    pub fn contains(&self, token: MioToken) -> bool {
        self.handlers.contains(token.0)
    }

    /// Number of active handlers.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Whether the handler slab is empty.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

/// Mio driver installer — generic over handler storage.
///
/// `S` is the handler storage type. Defaults to
/// `Box<dyn Handler<mio::event::Event>>`.
///
/// Consumed by [`WorldBuilder::install_driver`]. Registers a
/// [`MioDriver<S>`] resource and returns a [`MioPoller`] for poll-time use.
///
/// # Examples
///
/// ```ignore
/// // Defaults: 1024 events, 64 handlers
/// let poller = builder.install_driver(MioInstaller::new());
///
/// // Custom capacities
/// let poller = builder.install_driver(
///     MioInstaller::new()
///         .event_capacity(256)
///         .handler_capacity(32),
/// );
/// ```
pub struct MioInstaller<S = Box<dyn Handler<::mio::event::Event>>> {
    event_capacity: usize,
    handler_capacity: usize,
    _marker: PhantomData<S>,
}

impl<S> MioInstaller<S> {
    /// Creates a new mio installer with default capacities.
    ///
    /// Defaults:
    /// - `event_capacity`: 1024 (mio events buffer)
    /// - `handler_capacity`: 64 (slab pre-allocation)
    pub fn new() -> Self {
        MioInstaller {
            event_capacity: DEFAULT_EVENT_CAPACITY,
            handler_capacity: DEFAULT_HANDLER_CAPACITY,
            _marker: PhantomData,
        }
    }

    /// Set the mio events buffer capacity (default: 1024).
    ///
    /// This is the maximum number of readiness events returned per
    /// [`poll()`](MioPoller::poll) call. Events beyond this limit are
    /// deferred to the next poll.
    pub fn event_capacity(mut self, cap: usize) -> Self {
        self.event_capacity = cap;
        self
    }

    /// Set the initial handler slab pre-allocation (default: 64).
    ///
    /// The slab grows automatically if more handlers are inserted.
    /// Pre-allocating avoids reallocation during early setup.
    pub fn handler_capacity(mut self, cap: usize) -> Self {
        self.handler_capacity = cap;
        self
    }
}

impl<S> Default for MioInstaller<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Send + 'static> Installer for MioInstaller<S> {
    type Poller = MioPoller<S>;

    fn install(self, world: &mut WorldBuilder) -> MioPoller<S> {
        let poll = ::mio::Poll::new().expect("failed to create mio Poll");
        let handlers = ::slab::Slab::<S>::with_capacity(self.handler_capacity);
        let driver_id = world.register(MioDriver { poll, handlers });
        MioPoller {
            driver_id,
            events: ::mio::Events::with_capacity(self.event_capacity),
            buf: Vec::with_capacity(self.event_capacity),
        }
    }
}

/// Mio driver poller — poll-time handle.
///
/// Owns the `mio::Events` buffer and a drain buffer for the two-phase
/// poll pattern (drain handlers out, then fire).
pub struct MioPoller<S = Box<dyn Handler<::mio::event::Event>>> {
    driver_id: ResourceId,
    events: ::mio::Events,
    buf: Vec<(::mio::event::Event, S)>,
}

impl<S> std::fmt::Debug for MioPoller<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MioPoller")
            .field("driver_id", &self.driver_id)
            .field("buf_len", &self.buf.len())
            .finish()
    }
}

impl<S: DerefMut + Send + 'static> MioPoller<S>
where
    S::Target: Handler<::mio::event::Event>,
{
    /// Poll mio for readiness events and fire handlers.
    ///
    /// Three-phase:
    /// 1. Poll mio with the given timeout
    /// 2. Drain handlers from the slab into an internal buffer
    /// 3. Fire each handler with its event
    ///
    /// Returns [`Ok`] with the number of handlers fired on success.
    /// Returns [`Err`] if [`mio::Poll::poll`] fails; the error is
    /// propagated unchanged from the underlying mio call.
    ///
    /// Stale tokens (handler already removed) are silently skipped.
    ///
    /// # Re-registration
    ///
    /// Handlers are **moved out** of the slab before firing. A handler
    /// that wants to continue receiving events must re-insert a new
    /// handler and call `reregister()` on the source with the new token:
    ///
    /// ```ignore
    /// fn on_readable(mut driver: ResMut<MioDriver>, event: mio::event::Event) {
    ///     let stream: &mut TcpStream = /* ... */;
    ///     // ... read from stream ...
    ///
    ///     // Re-register for next event
    ///     let handler = on_readable.into_handler(/* registry */);
    ///     let new_token = driver.insert(Box::new(handler));
    ///     driver.registry()
    ///         .reregister(stream, new_token.into(), mio::Interest::READABLE)
    ///         .unwrap();
    /// }
    /// ```
    pub fn poll(&mut self, world: &mut World, timeout: Option<Duration>) -> io::Result<usize> {
        // 1. Poll mio
        // SAFETY: driver_id was produced by install() on the same builder.
        // Type matches (MioDriver<S>). We have &mut World.
        let driver = unsafe { world.get_mut::<MioDriver<S>>(self.driver_id) };
        driver.poll.poll(&mut self.events, timeout)?;

        // 2. Drain handlers — move out of slab for each ready token
        for event in &self.events {
            let key = event.token().0;
            if driver.handlers.contains(key) {
                let handler = driver.handlers.remove(key);
                self.buf.push((event.clone(), handler));
            }
        }

        // 3. Fire handlers
        let fired = self.buf.len();
        for (event, mut handler) in self.buf.drain(..) {
            world.next_sequence();
            handler.deref_mut().run(world, event);
        }

        Ok(fired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoHandler, ResMut, WorldBuilder};

    #[test]
    fn install_registers_driver() {
        let mut builder = WorldBuilder::new();
        let _poller: MioPoller = builder.install_driver(MioInstaller::new());
        let world = builder.build();
        assert!(world.contains::<MioDriver>());
    }

    #[test]
    fn poll_empty_returns_zero() {
        let mut builder = WorldBuilder::new();
        let mut poller: MioPoller = builder.install_driver(MioInstaller::new());
        let mut world = builder.build();
        let fired = poller
            .poll(&mut world, Some(Duration::from_millis(0)))
            .unwrap();
        assert_eq!(fired, 0);
    }

    #[test]
    fn waker_fires_handler() {
        let mut builder = WorldBuilder::new();
        builder.register::<bool>(false);
        let mut poller: MioPoller = builder.install_driver(MioInstaller::new());
        let mut world = builder.build();

        fn on_wake(mut flag: ResMut<bool>, _event: ::mio::event::Event) {
            *flag = true;
        }

        let handler = on_wake.into_handler(world.registry());
        let driver = world.resource_mut::<MioDriver>();
        let token = driver.insert(Box::new(handler));
        let waker = ::mio::Waker::new(driver.registry(), token.into()).unwrap();

        waker.wake().unwrap();

        let fired = poller
            .poll(&mut world, Some(Duration::from_millis(100)))
            .unwrap();
        assert_eq!(fired, 1);
        assert!(*world.resource::<bool>());
    }

    #[test]
    fn handler_fires_twice_with_waker() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut poller: MioPoller = builder.install_driver(MioInstaller::new());
        let mut world = builder.build();

        fn on_wake(mut counter: ResMut<u64>, _event: ::mio::event::Event) {
            *counter += 1;
        }

        // Create waker — only one allowed per Poll instance.
        let handler = on_wake.into_handler(world.registry());
        let driver = world.resource_mut::<MioDriver>();
        let token = driver.insert(Box::new(handler));
        let waker = ::mio::Waker::new(driver.registry(), token.into()).unwrap();

        waker.wake().unwrap();
        let fired = poller
            .poll(&mut world, Some(Duration::from_millis(100)))
            .unwrap();
        assert_eq!(fired, 1);
        assert_eq!(*world.resource::<u64>(), 1);

        // Re-insert handler. Slab reuses the freed slot, so the token
        // matches the waker's registration.
        let handler2 = on_wake.into_handler(world.registry());
        let driver = world.resource_mut::<MioDriver>();
        let token2 = driver.insert(Box::new(handler2));
        assert_eq!(token, token2, "slab must reuse freed slot");

        waker.wake().unwrap();
        let fired = poller
            .poll(&mut world, Some(Duration::from_millis(100)))
            .unwrap();
        assert_eq!(fired, 1);
        assert_eq!(*world.resource::<u64>(), 2);
    }

    #[test]
    fn cancel_before_fire() {
        let mut builder = WorldBuilder::new();
        builder.register::<bool>(false);
        let mut poller: MioPoller = builder.install_driver(MioInstaller::new());
        let mut world = builder.build();

        fn on_wake(mut flag: ResMut<bool>, _event: ::mio::event::Event) {
            *flag = true;
        }

        let handler = on_wake.into_handler(world.registry());
        let driver = world.resource_mut::<MioDriver>();
        let token = driver.insert(Box::new(handler));
        let waker = ::mio::Waker::new(driver.registry(), token.into()).unwrap();

        // Remove handler before waking
        let driver = world.resource_mut::<MioDriver>();
        let _removed = driver.remove(token);

        waker.wake().unwrap();

        // Poll — stale token, handler should NOT fire
        let fired = poller
            .poll(&mut world, Some(Duration::from_millis(100)))
            .unwrap();
        assert_eq!(fired, 0);
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn poll_advances_sequence() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut poller: MioPoller = builder.install_driver(MioInstaller::new());
        let mut world = builder.build();

        fn on_wake(mut counter: ResMut<u64>, _event: ::mio::event::Event) {
            *counter += 1;
        }

        let handler = on_wake.into_handler(world.registry());
        let driver = world.resource_mut::<MioDriver>();
        let token = driver.insert(Box::new(handler));
        let waker = ::mio::Waker::new(driver.registry(), token.into()).unwrap();

        waker.wake().unwrap();

        let seq_before = world.current_sequence();
        poller
            .poll(&mut world, Some(Duration::from_millis(100)))
            .unwrap();
        assert_eq!(world.current_sequence().0, seq_before.0 + 1);
    }

    #[test]
    fn stale_token_skipped() {
        let mut builder = WorldBuilder::new();
        let mut poller: MioPoller = builder.install_driver(MioInstaller::new());
        let mut world = builder.build();

        // Just poll with no sources — exercises the empty/stale path
        let fired = poller
            .poll(&mut world, Some(Duration::from_millis(0)))
            .unwrap();
        assert_eq!(fired, 0);
    }

    #[test]
    fn custom_capacities() {
        let mut builder = WorldBuilder::new();
        let _poller: MioPoller =
            builder.install_driver(MioInstaller::new().event_capacity(256).handler_capacity(32));
        let world = builder.build();
        assert!(world.contains::<MioDriver>());
    }
}
