//! Timer driver for nexus-rt.
//!
//! Integrates [`nexus_timer::Wheel`] as a driver following the
//! [`Installer`]/[`Plugin`](crate::Plugin) pattern. Handlers access the
//! timer wheel directly via `ResMut<Wheel<S>>` during dispatch — no
//! command queues, no side-channel communication.
//!
//! # Architecture
//!
//! - [`TimerInstaller`] is the installer — consumed at setup, registers the
//!   wheel into [`WorldBuilder`] and returns a [`TimerPoller`].
//! - [`TimerPoller`] is the poll-time handle. `poll(world, now)` drains
//!   expired timers and fires their handlers.
//! - Handlers reschedule themselves directly via `ResMut<Wheel<S>>`.
//!
//! # Timing
//!
//! The timer wheel records an **epoch** (`Instant`) at construction time
//! (inside [`TimerInstaller::install`]). All deadlines are converted to
//! integer ticks relative to this epoch:
//!
//! ```text
//! ticks = (deadline - epoch).as_nanos() / tick_ns
//! ```
//!
//! - **Default tick resolution**: 1ms (configurable via `WheelBuilder`).
//! - **Instants before the epoch** saturate to tick 0 (fire immediately).
//! - **Instants beyond the wheel's range** are clamped to the highest
//!   level's last slot (they fire eventually, not exactly on time).
//! - **Deadlines in the past** at poll time fire immediately — no "missed
//!   timer" error.
//!
//! The epoch is captured as `Instant::now()` during `install()`. This
//! means the wheel's zero point is the moment the driver is installed,
//! which is fine for monotonic deadlines derived from the same clock.
//!
//! # Examples
//!
//! ```ignore
//! use std::time::{Duration, Instant};
//! use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler};
//! use nexus_rt::timer::{TimerInstaller, TimerPoller, TimerWheel};
//!
//! fn on_timeout(mut state: ResMut<bool>, _poll_time: Instant) {
//!     *state = true;
//! }
//!
//! let mut builder = WorldBuilder::new();
//! builder.register::<bool>(false);
//! let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(256));
//! let mut world = builder.build();
//!
//! // Schedule a one-shot timer
//! let handler = on_timeout.into_handler(world.registry());
//! world.resource_mut::<TimerWheel>().schedule_forget(
//!     Instant::now() + Duration::from_millis(100),
//!     Box::new(handler),
//! );
//!
//! // In the poll loop:
//! // timer.poll(&mut world, Instant::now());
//! ```

use std::marker::PhantomData;
use std::ops::DerefMut;
use std::time::{Duration, Instant};

use nexus_timer::Wheel;

use crate::Handler;
use crate::driver::Installer;
use crate::world::{ResourceId, World, WorldBuilder};

/// Type alias for a timer wheel using boxed handlers (heap-allocated).
///
/// `Box<dyn Handler<Instant>>` — each timer entry is a type-erased handler
/// that receives the poll timestamp as its event.
pub type TimerWheel = Wheel<Box<dyn Handler<Instant>>>;

/// Type alias for a timer wheel using inline handler storage.
///
/// B256 = 256-byte inline buffer. Panics if a handler doesn't fit.
/// Realistic timer callbacks (0-2 resources + context) are 24-104 bytes,
/// so B256 provides comfortable headroom without a cache-line penalty
/// over B128 (SIMD memcpy).
#[cfg(feature = "smartptr")]
pub type InlineTimerWheel = Wheel<crate::FlatVirtual<Instant, nexus_smartptr::B256>>;

/// Type alias for a timer wheel using inline storage with heap fallback.
#[cfg(feature = "smartptr")]
pub type FlexTimerWheel = Wheel<crate::FlexVirtual<Instant, nexus_smartptr::B256>>;

/// Configuration trait for generic timer code.
///
/// ZST annotation type that bundles the handler storage type with a
/// wrapping function. Library code parameterized over `C: TimerConfig`
/// can schedule, cancel, and wrap handlers without knowing the concrete
/// storage strategy.
///
/// # Example
///
/// ```ignore
/// use std::time::Instant;
/// use nexus_rt::timer::{BoxedTimers, TimerConfig};
/// use nexus_rt::{Handler, World};
/// use nexus_timer::Wheel;
///
/// fn schedule_heartbeat<C: TimerConfig>(
///     world: &mut World,
///     handler: impl Handler<Instant> + 'static,
///     deadline: Instant,
/// ) {
///     world.resource_mut::<Wheel<C::Storage>>()
///         .schedule_forget(deadline, C::wrap(handler));
/// }
/// ```
pub trait TimerConfig: Send + 'static {
    /// The handler storage type (e.g. `Box<dyn Handler<Instant>>`).
    type Storage: DerefMut<Target = dyn Handler<Instant>> + Send + 'static;

    /// Wrap a concrete handler into the storage type.
    fn wrap(handler: impl Handler<Instant> + 'static) -> Self::Storage;
}

/// Boxed timer configuration — heap-allocates each handler.
///
/// This is the default and most flexible option. Zero-overhead for
/// `Option<Box<T>>` due to niche optimization.
pub struct BoxedTimers;

impl TimerConfig for BoxedTimers {
    type Storage = Box<dyn Handler<Instant>>;

    fn wrap(handler: impl Handler<Instant> + 'static) -> Self::Storage {
        Box::new(handler)
    }
}

/// Inline timer configuration — stores handlers in a fixed-size buffer.
///
/// Panics if a handler exceeds the buffer size (256 bytes).
/// Realistic timer callbacks (0-2 resources + context) are 24-104 bytes.
#[cfg(feature = "smartptr")]
pub struct InlineTimers;

#[cfg(feature = "smartptr")]
impl TimerConfig for InlineTimers {
    type Storage = crate::FlatVirtual<Instant, nexus_smartptr::B256>;

    fn wrap(handler: impl Handler<Instant> + 'static) -> Self::Storage {
        let ptr: *const dyn Handler<Instant> = &handler;
        // SAFETY: ptr's metadata (vtable) corresponds to handler's concrete type.
        unsafe { nexus_smartptr::Flat::new_raw(handler, ptr) }
    }
}

/// Flex timer configuration — inline with heap fallback.
///
/// Stores inline if the handler fits in 256 bytes, otherwise
/// heap-allocates. No panics.
#[cfg(feature = "smartptr")]
pub struct FlexTimers;

#[cfg(feature = "smartptr")]
impl TimerConfig for FlexTimers {
    type Storage = crate::FlexVirtual<Instant, nexus_smartptr::B256>;

    fn wrap(handler: impl Handler<Instant> + 'static) -> Self::Storage {
        let ptr: *const dyn Handler<Instant> = &handler;
        // SAFETY: ptr's metadata (vtable) corresponds to handler's concrete type.
        unsafe { nexus_smartptr::Flex::new_raw(handler, ptr) }
    }
}

/// Timer driver installer — generic over handler storage.
///
/// `S` is the handler storage type (e.g. `Box<dyn Handler<Instant>>` or
/// `FlatVirtual<Instant, B256>`). Defaults to `Box<dyn Handler<Instant>>`.
///
/// Consumed by [`WorldBuilder::install_driver`]. Registers a
/// [`Wheel<S>`] resource and returns a [`TimerPoller`] for poll-time use.
pub struct TimerInstaller<S = Box<dyn Handler<Instant>>> {
    chunk_capacity: usize,
    wheel_config: Option<nexus_timer::WheelBuilder>,
    _marker: PhantomData<S>,
}

impl<S> TimerInstaller<S> {
    /// Creates a new timer driver installer with the given slab chunk capacity.
    ///
    /// Uses default wheel configuration (1ms tick, 64 slots, 7 levels).
    pub fn new(chunk_capacity: usize) -> Self {
        TimerInstaller {
            chunk_capacity,
            wheel_config: None,
            _marker: PhantomData,
        }
    }

    /// Sets a custom wheel configuration.
    ///
    /// Use this when the default 1ms tick resolution isn't sufficient.
    /// The `chunk_capacity` from [`new`](Self::new) is still used for
    /// slab allocation.
    pub fn with_config(mut self, config: nexus_timer::WheelBuilder) -> Self {
        self.wheel_config = Some(config);
        self
    }
}

/// Timer driver poller — generic over handler storage.
///
/// Returned by [`TimerInstaller::install`]. Holds a pre-resolved
/// [`ResourceId`] for the wheel and a reusable drain buffer.
pub struct TimerPoller<S = Box<dyn Handler<Instant>>> {
    wheel_id: ResourceId,
    buf: Vec<S>,
}

impl<S> std::fmt::Debug for TimerPoller<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimerPoller")
            .field("wheel_id", &self.wheel_id)
            .field("buf_len", &self.buf.len())
            .finish()
    }
}

impl<S: Send + 'static> Installer for TimerInstaller<S> {
    type Poller = TimerPoller<S>;

    fn install(self, world: &mut WorldBuilder) -> TimerPoller<S> {
        let now = Instant::now();
        let cap = self.chunk_capacity;
        let wheel = self.wheel_config.map_or_else(
            || Wheel::unbounded(cap, now),
            |c| c.unbounded(cap).build(now),
        );
        let wheel_id = world.register::<Wheel<S>>(wheel);
        TimerPoller {
            wheel_id,
            buf: Vec::new(),
        }
    }
}

impl<S: DerefMut + Send + 'static> TimerPoller<S>
where
    S::Target: Handler<Instant>,
{
    /// Poll expired timers — drain from wheel, fire each handler, done.
    ///
    /// Each handler receives `now` as its event. Handlers that need to
    /// reschedule themselves do so directly via `ResMut<Wheel<S>>`.
    ///
    /// Returns the number of timers fired.
    pub fn poll(&mut self, world: &mut World, now: Instant) -> usize {
        // 1. Drain expired from wheel (get_mut via pre-resolved ID)
        // SAFETY: wheel_id was produced by install() on the same builder.
        // Type matches (Wheel<S>). No aliases — we have &mut World.
        let wheel = unsafe { world.get_mut::<Wheel<S>>(self.wheel_id) };
        wheel.poll(now, &mut self.buf);
        let fired = self.buf.len();

        // 2. Fire each handler — handler is responsible for rescheduling
        for mut handler in self.buf.drain(..) {
            world.next_sequence();
            handler.deref_mut().run(world, now);
            // handler dropped if it didn't reschedule itself
        }

        fired
    }

    /// Earliest deadline in the wheel.
    pub fn next_deadline(&self, world: &World) -> Option<Instant> {
        // SAFETY: wheel_id from install(). Type matches. &World = shared access.
        let wheel = unsafe { world.get::<Wheel<S>>(self.wheel_id) };
        wheel.next_deadline()
    }

    /// Number of active timers.
    pub fn len(&self, world: &World) -> usize {
        // SAFETY: wheel_id from install(). Type matches. &World = shared access.
        let wheel = unsafe { world.get::<Wheel<S>>(self.wheel_id) };
        wheel.len()
    }

    /// Whether the wheel is empty.
    pub fn is_empty(&self, world: &World) -> bool {
        // SAFETY: wheel_id from install(). Type matches. &World = shared access.
        let wheel = unsafe { world.get::<Wheel<S>>(self.wheel_id) };
        wheel.is_empty()
    }
}

/// Periodic timer wrapper — automatically reschedules after each firing.
///
/// Wraps any handler storage and re-inserts itself into the wheel after
/// each `run()` call. The inner handler fires, then `Periodic` wraps it
/// back up and schedules `now + interval`.
///
/// Uses `Option` internally to move the inner handler out of `&mut self`
/// during dispatch. For `Box<dyn Handler>`, this is zero-cost due to
/// niche optimization (`Option<Box<T>>` is pointer-sized).
///
/// # Cancellation
///
/// If the periodic timer is cancelled (via [`Wheel::cancel`]) or dropped
/// during shutdown, the inner handler is dropped normally — no leak.
///
/// # Example
///
/// ```ignore
/// use std::time::{Duration, Instant};
/// use nexus_rt::{IntoHandler, ResMut};
/// use nexus_rt::timer::{Periodic, TimerWheel};
///
/// fn heartbeat(mut counter: ResMut<u64>, _now: Instant) {
///     *counter += 1;
/// }
///
/// let handler = heartbeat.into_handler(world.registry());
/// let periodic = Periodic::boxed(handler, Duration::from_millis(100));
/// world.resource_mut::<TimerWheel>()
///     .schedule_forget(Instant::now(), Box::new(periodic));
/// ```
pub struct Periodic<C: TimerConfig = BoxedTimers> {
    inner: Option<C::Storage>,
    interval: Duration,
    _config: PhantomData<C>,
}

impl<C: TimerConfig> std::fmt::Debug for Periodic<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Periodic")
            .field("has_inner", &self.inner.is_some())
            .field("interval", &self.interval)
            .finish()
    }
}

impl Periodic<BoxedTimers> {
    /// Create a periodic wrapper using boxed handler storage.
    ///
    /// Convenience constructor — equivalent to `Periodic::<BoxedTimers>::new(...)`.
    pub fn boxed(handler: impl Handler<Instant> + 'static, interval: Duration) -> Self {
        Periodic {
            inner: Some(Box::new(handler)),
            interval,
            _config: PhantomData,
        }
    }
}

impl<C: TimerConfig> Periodic<C> {
    /// Create a periodic wrapper with the given config's storage strategy.
    pub fn new(storage: C::Storage, interval: Duration) -> Self {
        Periodic {
            inner: Some(storage),
            interval,
            _config: PhantomData,
        }
    }

    /// Create a periodic wrapper, wrapping the handler via [`TimerConfig::wrap`].
    pub fn wrap(handler: impl Handler<Instant> + 'static, interval: Duration) -> Self {
        Periodic {
            inner: Some(C::wrap(handler)),
            interval,
            _config: PhantomData,
        }
    }

    /// Returns the repetition interval.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Unwrap the inner handler storage, if present.
    ///
    /// Returns `None` only if the periodic has already fired and not yet
    /// been re-wrapped (transient state during `Handler::run`).
    pub fn into_inner(self) -> Option<C::Storage> {
        self.inner
    }
}

impl<C: TimerConfig> Handler<Instant> for Periodic<C> {
    fn run(&mut self, world: &mut World, now: Instant) {
        let mut inner = self
            .inner
            .take()
            .expect("periodic handler already consumed");

        // Fire the inner handler
        inner.deref_mut().run(world, now);

        // Re-wrap and reschedule
        let next = Periodic::<C> {
            inner: Some(inner),
            interval: self.interval,
            _config: PhantomData,
        };
        let deadline = now + self.interval;
        world
            .resource_mut::<Wheel<C::Storage>>()
            .schedule_forget(deadline, C::wrap(next));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoCallback, IntoHandler, RegistryRef, ResMut, WorldBuilder};
    use std::time::Duration;

    #[test]
    fn install_registers_wheel() {
        let mut builder = WorldBuilder::new();
        let _handle: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let world = builder.build();
        assert!(world.contains::<TimerWheel>());
    }

    #[test]
    fn poll_empty_returns_zero() {
        let mut builder = WorldBuilder::new();
        let mut handle: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();
        assert_eq!(handle.poll(&mut world, Instant::now()), 0);
    }

    #[test]
    fn one_shot_fires() {
        let mut builder = WorldBuilder::new();
        builder.register::<bool>(false);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn on_timeout(mut flag: ResMut<bool>, _now: Instant) {
            *flag = true;
        }

        let handler = on_timeout.into_handler(world.registry());
        let now = Instant::now();
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(now, Box::new(handler));

        assert!(!*world.resource::<bool>());
        let fired = timer.poll(&mut world, now);
        assert_eq!(fired, 1);
        assert!(*world.resource::<bool>());
    }

    #[test]
    fn expired_timer_fires_accumulated() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn inc(mut counter: ResMut<u64>, _now: Instant) {
            *counter += 1;
        }

        let now = Instant::now();
        let past = now - Duration::from_millis(10);

        for _ in 0..3 {
            let h = inc.into_handler(world.registry());
            world
                .resource_mut::<TimerWheel>()
                .schedule_forget(past, Box::new(h));
        }

        let fired = timer.poll(&mut world, now);
        assert_eq!(fired, 3);
        assert_eq!(*world.resource::<u64>(), 3);
    }

    #[test]
    fn future_timer_does_not_fire() {
        let mut builder = WorldBuilder::new();
        builder.register::<bool>(false);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn on_timeout(mut flag: ResMut<bool>, _now: Instant) {
            *flag = true;
        }

        let now = Instant::now();
        let future = now + Duration::from_secs(60);
        let h = on_timeout.into_handler(world.registry());
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(future, Box::new(h));

        let fired = timer.poll(&mut world, now);
        assert_eq!(fired, 0);
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn next_deadline_reports_earliest() {
        let mut builder = WorldBuilder::new();
        let timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        let now = Instant::now();
        let early = now + Duration::from_millis(50);
        let late = now + Duration::from_millis(200);

        fn noop(_now: Instant) {}

        let h1 = noop.into_handler(world.registry());
        let h2 = noop.into_handler(world.registry());
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(late, Box::new(h1));
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(early, Box::new(h2));

        let deadline = timer.next_deadline(&world);
        assert!(deadline.is_some());
        // Deadline should be <= early (timer wheel rounds to tick granularity)
        assert!(deadline.unwrap() <= early + Duration::from_millis(1));
    }

    #[test]
    fn len_tracks_active_timers() {
        let mut builder = WorldBuilder::new();
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        assert_eq!(timer.len(&world), 0);
        assert!(timer.is_empty(&world));

        let now = Instant::now();
        fn noop(_now: Instant) {}

        let h = noop.into_handler(world.registry());
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(now, Box::new(h));

        assert_eq!(timer.len(&world), 1);
        assert!(!timer.is_empty(&world));

        timer.poll(&mut world, now);
        assert_eq!(timer.len(&world), 0);
    }

    #[test]
    fn self_rescheduling_callback() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn periodic(
            ctx: &mut Duration,
            mut counter: ResMut<u64>,
            mut wheel: ResMut<TimerWheel>,
            reg: RegistryRef,
            now: Instant,
        ) {
            *counter += 1;
            if *counter < 3 {
                let interval = *ctx;
                let next = periodic.into_callback(interval, &reg);
                wheel.schedule_forget(now + interval, Box::new(next));
            }
        }

        let now = Instant::now();
        let interval = Duration::from_millis(1);
        let cb = periodic.into_callback(interval, world.registry());
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(now, Box::new(cb));

        // Fire first
        timer.poll(&mut world, now);
        assert_eq!(*world.resource::<u64>(), 1);

        // Fire second (rescheduled)
        timer.poll(&mut world, now + interval);
        assert_eq!(*world.resource::<u64>(), 2);

        // Fire third (rescheduled again, but won't reschedule since counter >= 3)
        timer.poll(&mut world, now + interval * 2);
        assert_eq!(*world.resource::<u64>(), 3);

        // No more timers
        assert!(timer.is_empty(&world));
    }

    #[test]
    fn cancellable_timer() {
        let mut builder = WorldBuilder::new();
        builder.register::<bool>(false);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn on_fire(mut flag: ResMut<bool>, _now: Instant) {
            *flag = true;
        }

        let now = Instant::now();
        let deadline = now + Duration::from_millis(100);
        let h = on_fire.into_handler(world.registry());
        let cancel_handle = world
            .resource_mut::<TimerWheel>()
            .schedule(deadline, Box::new(h));

        // Cancel before firing
        let cancelled = world.resource_mut::<TimerWheel>().cancel(cancel_handle);
        assert!(cancelled.is_some());

        // Poll — nothing fires
        let fired = timer.poll(&mut world, deadline);
        assert_eq!(fired, 0);
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn poll_advances_sequence() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn inc(mut counter: ResMut<u64>, _now: Instant) {
            *counter += 1;
        }

        let now = Instant::now();
        let h1 = inc.into_handler(world.registry());
        let h2 = inc.into_handler(world.registry());
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(now, Box::new(h1));
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(now, Box::new(h2));

        let seq_before = world.current_sequence();
        timer.poll(&mut world, now);
        // Two handlers fired, two next_sequence calls
        assert_eq!(world.current_sequence().0, seq_before.0 + 2);
    }

    #[test]
    fn reschedule_timer() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn on_fire(mut counter: ResMut<u64>, _now: Instant) {
            *counter += 1;
        }

        let now = Instant::now();
        let h = on_fire.into_handler(world.registry());
        let handle = world
            .resource_mut::<TimerWheel>()
            .schedule(now + Duration::from_millis(100), Box::new(h));

        // Reschedule to earlier
        let handle = world
            .resource_mut::<TimerWheel>()
            .reschedule(handle, now + Duration::from_millis(50));

        // Should NOT fire at 40ms
        let fired = timer.poll(&mut world, now + Duration::from_millis(40));
        assert_eq!(fired, 0);
        assert_eq!(*world.resource::<u64>(), 0);

        // Should fire at 55ms
        let fired = timer.poll(&mut world, now + Duration::from_millis(55));
        assert_eq!(fired, 1);
        assert_eq!(*world.resource::<u64>(), 1);

        // Clean up zombie handle
        world.resource_mut::<TimerWheel>().cancel(handle);
    }

    #[test]
    fn periodic_fires_repeatedly() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn tick(mut counter: ResMut<u64>, _now: Instant) {
            *counter += 1;
        }

        let now = Instant::now();
        let interval = Duration::from_millis(10);
        let handler = tick.into_handler(world.registry());
        let periodic = Periodic::boxed(handler, interval);
        world
            .resource_mut::<TimerWheel>()
            .schedule_forget(now, Box::new(periodic));

        // First firing
        timer.poll(&mut world, now);
        assert_eq!(*world.resource::<u64>(), 1);

        // Second firing (rescheduled to now + 10ms)
        timer.poll(&mut world, now + interval);
        assert_eq!(*world.resource::<u64>(), 2);

        // Third firing (rescheduled to now + 20ms)
        timer.poll(&mut world, now + interval * 2);
        assert_eq!(*world.resource::<u64>(), 3);

        // Still active — periodic never stops on its own
        assert!(!timer.is_empty(&world));
    }

    #[test]
    fn periodic_cancel_drops_inner() {
        let mut builder = WorldBuilder::new();
        builder.register::<bool>(false);
        let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let mut world = builder.build();

        fn on_fire(mut flag: ResMut<bool>, _now: Instant) {
            *flag = true;
        }

        let now = Instant::now();
        let handler = on_fire.into_handler(world.registry());
        let periodic = Periodic::boxed(handler, Duration::from_millis(50));
        let handle = world
            .resource_mut::<TimerWheel>()
            .schedule(now + Duration::from_millis(50), Box::new(periodic));

        // Cancel before it fires
        let cancelled = world.resource_mut::<TimerWheel>().cancel(handle);
        assert!(cancelled.is_some());

        // Poll — nothing fires
        let fired = timer.poll(&mut world, now + Duration::from_millis(100));
        assert_eq!(fired, 0);
        assert!(!*world.resource::<bool>());
    }

    #[test]
    fn periodic_into_inner_recovers_handler() {
        let mut builder = WorldBuilder::new();
        let _timer: TimerPoller = builder.install_driver(TimerInstaller::new(64));
        let world = builder.build();

        fn noop(_now: Instant) {}

        let handler = noop.into_handler(world.registry());
        let periodic = Periodic::boxed(handler, Duration::from_millis(10));
        assert!(periodic.into_inner().is_some());
    }
}
