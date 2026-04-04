//! Timer driver backed by nexus-timer wheel.
//!
//! O(1) insert and cancel via hierarchical timer wheel. Expired timers
//! are collected into a pre-allocated buffer and their wakers fired.
//! Integrates with the mio poll timeout — the nearest deadline
//! determines how long epoll blocks.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use nexus_timer::{Wheel, WheelBuilder};

// =============================================================================
// TimerDriver — owned by Runtime
// =============================================================================

/// Timer wheel driver. O(1) insert, O(1) cancel, no-cascade poll.
pub(crate) struct TimerDriver {
    wheel: Wheel<Waker>,
    /// Pre-allocated buffer for expired wakers. Reused across cycles.
    expired: Vec<Waker>,
}

impl TimerDriver {
    pub(crate) fn new(capacity: usize) -> Self {
        let now = Instant::now();
        let wheel = WheelBuilder::default()
            .unbounded(capacity)
            .build(now);
        Self {
            wheel,
            expired: Vec::with_capacity(64),
        }
    }

    /// Schedule a deadline with a waker to call on expiry.
    /// Fire-and-forget — no handle returned (the Sleep future
    /// doesn't need to cancel).
    pub(crate) fn schedule(&mut self, deadline: Instant, waker: Waker) {
        self.wheel.schedule_forget(deadline, waker);
    }

    /// Returns the nearest deadline, or `None` if no timers are pending.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.wheel.next_deadline()
    }

    /// Drain all expired timers and wake their tasks.
    ///
    /// Returns the number of timers fired.
    pub(crate) fn fire_expired(&mut self, now: Instant) -> usize {
        self.expired.clear();
        let fired = self.wheel.poll(now, &mut self.expired);
        for waker in self.expired.drain(..) {
            waker.wake();
        }
        fired
    }
}

// =============================================================================
// TimerHandle — Copy handle for tasks
// =============================================================================

/// [`Copy`] handle for scheduling timers from async tasks.
#[derive(Clone, Copy)]
pub struct TimerHandle {
    driver: *mut TimerDriver,
}

impl TimerHandle {
    pub(crate) fn new(driver: &mut TimerDriver) -> Self {
        Self {
            driver: driver as *mut TimerDriver,
        }
    }

    /// Create a [`Sleep`] future that completes after `duration`.
    pub fn sleep(&self, duration: Duration) -> Sleep {
        Sleep {
            deadline: Instant::now() + duration,
            driver: self.driver,
            registered: false,
        }
    }

    /// Create a [`Sleep`] future that completes at `deadline`.
    pub fn sleep_until(&self, deadline: Instant) -> Sleep {
        Sleep {
            deadline,
            driver: self.driver,
            registered: false,
        }
    }
}

// =============================================================================
// Sleep future
// =============================================================================

/// Future that completes when a deadline expires.
///
/// On first poll, registers the deadline with the timer wheel. On
/// subsequent polls, checks if the deadline has passed. The timer
/// driver wakes the task when the deadline expires.
pub struct Sleep {
    deadline: Instant,
    driver: *mut TimerDriver,
    registered: bool,
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if Instant::now() >= self.deadline {
            return Poll::Ready(());
        }

        if !self.registered {
            // SAFETY: driver pointer is valid (Runtime lifetime).
            let driver = unsafe { &mut *self.driver };
            driver.schedule(self.deadline, cx.waker().clone());
            self.registered = true;
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{RawWaker, RawWakerVTable};

    fn noop_waker() -> Waker {
        fn noop(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }

    #[test]
    fn timer_driver_fire_expired() {
        let mut driver = TimerDriver::new(64);
        let now = Instant::now();
        let waker = noop_waker();

        driver.schedule(now - Duration::from_millis(10), waker.clone());
        driver.schedule(now + Duration::from_secs(100), waker);

        let fired = driver.fire_expired(now);
        assert_eq!(fired, 1);
        assert!(driver.next_deadline().unwrap() > now);
    }

    #[test]
    fn timer_driver_next_deadline() {
        let mut driver = TimerDriver::new(64);
        assert!(driver.next_deadline().is_none());

        let now = Instant::now();
        let soon = now + Duration::from_millis(10);
        let later = now + Duration::from_millis(100);
        let waker = noop_waker();

        driver.schedule(later, waker.clone());
        driver.schedule(soon, waker);

        let next = driver.next_deadline().unwrap();
        // Timer wheel has tick-resolution quantization, so check within 2ms.
        assert!(next <= soon + Duration::from_millis(2));
    }
}
