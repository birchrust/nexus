//! Timer handle — proof that a timer was scheduled with the intent to cancel.
//!
//! A `TimerHandle<T>` is an 8-byte, move-only token returned by `schedule()`.
//! It must be explicitly consumed via `wheel.cancel(handle)` or
//! `wheel.free(handle)`. Dropping a handle without consuming it is a
//! programming error (caught by `debug_assert!` in debug builds).

use std::fmt;
use std::marker::PhantomData;

use crate::entry::EntryPtr;

/// Handle to a scheduled timer.
///
/// Returned by [`TimerWheel::schedule`](crate::TimerWheel::schedule) and
/// [`TimerWheel::try_schedule`](crate::TimerWheel::try_schedule). Must be
/// consumed by one of:
///
/// - [`TimerWheel::cancel`](crate::TimerWheel::cancel) — unlinks from the
///   wheel and extracts the value.
/// - [`TimerWheel::free`](crate::TimerWheel::free) — releases the handle,
///   converting to fire-and-forget.
///
/// Dropping without consuming is a programming error (`debug_assert!` fires).
///
/// # Size
///
/// 8 bytes (one pointer). `!Send`, `!Sync`, `!Clone`, `!Copy`.
#[must_use = "handles must be consumed via cancel() or free(), dropping leaks the timer slot"]
pub struct TimerHandle<T> {
    pub(crate) ptr: EntryPtr<T>,
    // !Send, !Sync
    _marker: PhantomData<*const ()>,
}

impl<T> TimerHandle<T> {
    /// Creates a new handle from an entry pointer.
    #[inline]
    pub(crate) fn new(ptr: EntryPtr<T>) -> Self {
        TimerHandle {
            ptr,
            _marker: PhantomData,
        }
    }
}

impl<T> Drop for TimerHandle<T> {
    fn drop(&mut self) {
        debug_assert!(
            false,
            "TimerHandle dropped without being consumed — call wheel.cancel() or wheel.free()"
        );
    }
}

impl<T> fmt::Debug for TimerHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimerHandle")
            .field("ptr", &self.ptr)
            .finish()
    }
}
