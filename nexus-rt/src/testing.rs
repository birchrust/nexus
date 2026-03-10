//! Testing utilities for nexus-rt handlers and timer drivers.
//!
//! Two tiers of testing infrastructure:
//!
//! - [`TestHarness`] — dispatch events through handlers directly,
//!   auto-advancing sequence numbers, with world access for assertions.
//! - [`TestTimerDriver`] — wraps [`TimerPoller`](crate::timer::TimerPoller)
//!   with virtual time control for deterministic timer testing.
//!
//! Always available (no feature gate).

use crate::Handler;
use crate::world::{Registry, World, WorldBuilder};

// =============================================================================
// TestHarness
// =============================================================================

/// Minimal test harness for handler dispatch.
///
/// Owns a [`World`] and auto-advances the sequence number before each
/// dispatch. Designed for unit testing handlers without wiring up real
/// drivers.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, IntoHandler};
/// use nexus_rt::testing::TestHarness;
///
/// fn accumulate(mut counter: ResMut<u64>, event: u64) {
///     *counter += event;
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
/// let mut harness = TestHarness::new(builder);
///
/// let mut handler = accumulate.into_handler(harness.registry());
/// harness.dispatch(&mut handler, 10u64);
/// harness.dispatch(&mut handler, 5u64);
///
/// assert_eq!(*harness.world().resource::<u64>(), 15);
/// ```
pub struct TestHarness {
    world: World,
}

impl TestHarness {
    /// Build a test harness from a [`WorldBuilder`].
    pub fn new(builder: WorldBuilder) -> Self {
        Self {
            world: builder.build(),
        }
    }

    /// Registry access for creating handlers after build.
    pub fn registry(&self) -> &Registry {
        self.world.registry()
    }

    /// Advance sequence and dispatch one event through a handler.
    pub fn dispatch<E>(&mut self, handler: &mut impl Handler<E>, event: E) {
        self.world.next_sequence();
        handler.run(&mut self.world, event);
    }

    /// Dispatch multiple events sequentially, advancing sequence per event.
    pub fn dispatch_many<E>(
        &mut self,
        handler: &mut impl Handler<E>,
        events: impl IntoIterator<Item = E>,
    ) {
        for event in events {
            self.dispatch(handler, event);
        }
    }

    /// Read-only world access for assertions.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Mutable world access (e.g. to stamp resources manually).
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }
}

// =============================================================================
// TestTimerDriver (behind `timer` feature)
// =============================================================================

#[cfg(feature = "timer")]
mod timer_driver {
    use std::ops::DerefMut;
    use std::time::{Duration, Instant};

    use crate::Handler;
    use crate::timer::TimerPoller;
    use crate::world::World;

    /// Virtual-time wrapper around [`TimerPoller`] for deterministic timer
    /// testing.
    ///
    /// Captures `Instant::now()` at construction as the starting time.
    /// [`advance`](Self::advance) and [`set_now`](Self::set_now) control
    /// the virtual clock. [`poll`](Self::poll) delegates to the inner
    /// poller using the virtual time.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::time::Duration;
    /// use nexus_rt::{WorldBuilder, ResMut, IntoHandler};
    /// use nexus_rt::timer::{TimerInstaller, TimerWheel};
    /// use nexus_rt::testing::TestTimerDriver;
    ///
    /// let mut builder = WorldBuilder::new();
    /// builder.register::<bool>(false);
    /// let poller = builder.install_driver(TimerInstaller::new(256));
    /// let mut timer = TestTimerDriver::new(poller);
    /// let mut world = builder.build();
    ///
    /// fn on_fire(mut flag: ResMut<bool>, _now: std::time::Instant) {
    ///     *flag = true;
    /// }
    ///
    /// let handler = on_fire.into_handler(world.registry());
    /// let deadline = timer.now() + Duration::from_millis(100);
    /// world.resource_mut::<TimerWheel>()
    ///     .schedule_forget(deadline, Box::new(handler));
    ///
    /// timer.advance(Duration::from_millis(150));
    /// let fired = timer.poll(&mut world);
    /// assert_eq!(fired, 1);
    /// assert!(*world.resource::<bool>());
    /// ```
    pub struct TestTimerDriver<S: 'static = Box<dyn Handler<Instant>>> {
        poller: TimerPoller<S>,
        now: Instant,
    }

    impl<S: DerefMut + Send + 'static> TestTimerDriver<S>
    where
        S::Target: Handler<Instant>,
    {
        /// Wrap an installed [`TimerPoller`]. Captures `Instant::now()` as
        /// the starting time.
        pub fn new(poller: TimerPoller<S>) -> Self {
            Self {
                poller,
                now: Instant::now(),
            }
        }

        /// Current virtual time.
        pub fn now(&self) -> Instant {
            self.now
        }

        /// Advance virtual time by a duration.
        pub fn advance(&mut self, duration: Duration) {
            self.now += duration;
        }

        /// Set virtual time to a specific instant.
        pub fn set_now(&mut self, now: Instant) {
            self.now = now;
        }

        /// Poll expired timers at the current virtual time.
        ///
        /// Delegates to [`TimerPoller::poll`] with the virtual time.
        /// Returns the number of timers fired.
        pub fn poll(&mut self, world: &mut World) -> usize {
            self.poller.poll(world, self.now)
        }

        /// Earliest deadline in the wheel.
        pub fn next_deadline(&self, world: &World) -> Option<Instant> {
            self.poller.next_deadline(world)
        }

        /// Number of active timers.
        pub fn len(&self, world: &World) -> usize {
            self.poller.len(world)
        }

        /// Whether the wheel is empty.
        pub fn is_empty(&self, world: &World) -> bool {
            self.poller.is_empty(world)
        }
    }
}

#[cfg(feature = "timer")]
pub use timer_driver::TestTimerDriver;

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoHandler, ResMut};

    // -- TestHarness tests ------------------------------------------------

    fn accumulate(mut counter: ResMut<u64>, event: u64) {
        *counter += event;
    }

    #[test]
    fn dispatch_advances_sequence() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut harness = TestHarness::new(builder);

        let seq_before = harness.world().current_sequence();
        let mut handler = accumulate.into_handler(harness.registry());
        harness.dispatch(&mut handler, 1u64);
        assert_eq!(harness.world().current_sequence().0, seq_before.0 + 1);
    }

    #[test]
    fn dispatch_runs_handler() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut harness = TestHarness::new(builder);

        let mut handler = accumulate.into_handler(harness.registry());
        harness.dispatch(&mut handler, 10u64);
        assert_eq!(*harness.world().resource::<u64>(), 10);
    }

    #[test]
    fn dispatch_many_sequential() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut harness = TestHarness::new(builder);

        let seq_before = harness.world().current_sequence();
        let mut handler = accumulate.into_handler(harness.registry());
        harness.dispatch_many(&mut handler, [10u64, 5, 3]);
        assert_eq!(*harness.world().resource::<u64>(), 18);
        assert_eq!(harness.world().current_sequence().0, seq_before.0 + 3);
    }

    #[test]
    fn world_access() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let mut harness = TestHarness::new(builder);

        assert_eq!(*harness.world().resource::<u64>(), 42);

        *harness.world_mut().resource_mut::<u64>() = 99;
        assert_eq!(*harness.world().resource::<u64>(), 99);
    }

    // -- TestTimerDriver tests --------------------------------------------

    #[cfg(feature = "timer")]
    mod timer_tests {
        use crate::testing::TestTimerDriver;
        use crate::timer::{TimerInstaller, TimerPoller, TimerWheel};
        use crate::{IntoHandler, ResMut, WorldBuilder};
        use std::time::{Duration, Instant};

        fn set_flag(mut flag: ResMut<bool>, _now: Instant) {
            *flag = true;
        }

        #[test]
        fn advance_moves_time() {
            let mut builder = WorldBuilder::new();
            let poller: TimerPoller = builder.install_driver(TimerInstaller::new());
            let mut timer = TestTimerDriver::new(poller);

            let start = timer.now();
            timer.advance(Duration::from_millis(100));
            assert_eq!(timer.now(), start + Duration::from_millis(100));
        }

        #[test]
        fn poll_fires_expired() {
            let mut builder = WorldBuilder::new();
            builder.register::<bool>(false);
            let poller: TimerPoller = builder.install_driver(TimerInstaller::new());
            let mut timer = TestTimerDriver::new(poller);
            let mut world = builder.build();

            let deadline = timer.now() + Duration::from_millis(100);
            let handler = set_flag.into_handler(world.registry());
            world
                .resource_mut::<TimerWheel>()
                .schedule_forget(deadline, Box::new(handler));

            timer.advance(Duration::from_millis(150));
            let fired = timer.poll(&mut world);
            assert_eq!(fired, 1);
            assert!(*world.resource::<bool>());
        }

        #[test]
        fn poll_skips_future() {
            let mut builder = WorldBuilder::new();
            builder.register::<bool>(false);
            let poller: TimerPoller = builder.install_driver(TimerInstaller::new());
            let mut timer = TestTimerDriver::new(poller);
            let mut world = builder.build();

            let deadline = timer.now() + Duration::from_secs(60);
            let handler = set_flag.into_handler(world.registry());
            world
                .resource_mut::<TimerWheel>()
                .schedule_forget(deadline, Box::new(handler));

            let fired = timer.poll(&mut world);
            assert_eq!(fired, 0);
            assert!(!*world.resource::<bool>());
        }

        #[test]
        fn set_now_overrides() {
            let mut builder = WorldBuilder::new();
            let poller: TimerPoller = builder.install_driver(TimerInstaller::new());
            let mut timer = TestTimerDriver::new(poller);

            let target = timer.now() + Duration::from_secs(999);
            timer.set_now(target);
            assert_eq!(timer.now(), target);
        }
    }
}
