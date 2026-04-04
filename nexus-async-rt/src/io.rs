//! IO driver backed by mio.
//!
//! The [`IoDriver`] owns a `mio::Poll` instance and a token→waker mapping.
//! When mio reports readiness on a token, the associated task is woken
//! (pointer pushed to the ready queue via the waker).
//!
//! Tasks interact with IO through [`IoHandle`], a [`Copy`] handle that
//! provides source registration and deregistration.
//!
//! # Token lifecycle
//!
//! 1. Task calls `io.register(&mut source, interest)` → gets a `mio::Token`
//! 2. Runtime calls `mio::Poll::poll` → readiness events arrive
//! 3. For each event, the driver wakes the associated task
//! 4. Task calls `io.deregister(&mut source)` when done
//!
//! Tokens are reused via a freelist. Stale wakeups (token reused after
//! deregister) produce spurious wakeups — futures must tolerate this
//! per the async contract.

use std::io;
use std::time::Duration;

use std::sync::Arc;

use mio::event::Source;
use mio::{Events, Interest, Poll, Token};

use crate::waker;

// =============================================================================
// IoDriver — owned by Runtime
// =============================================================================

/// Reserved token for the mio::Waker (used to break out of epoll_wait
/// when the root future or spawned tasks need attention).
const WAKER_TOKEN: Token = Token(usize::MAX);

/// Mio-backed IO driver. Owns the `Poll` instance and token→waker map.
pub(crate) struct IoDriver {
    /// Mio poll instance. Wraps epoll/kqueue.
    poll: Poll,

    /// Pre-allocated events buffer.
    events: Events,

    /// Mio waker for breaking out of `Poll::poll` from outside the
    /// poll loop (e.g., root future's waker, or spawned task waker
    /// firing from a callback).
    mio_waker: Arc<mio::Waker>,

    /// Token → task pointer. Indexed by `Token.0`.
    /// `None` = vacant slot (in freelist).
    /// `Some(ptr)` = task to wake on readiness.
    wakers: Vec<Option<*mut u8>>,

    /// Head of the token freelist. `usize::MAX` = empty.
    free_head: usize,
}

/// Sentinel for empty freelist.
const NO_FREE: usize = usize::MAX;

impl IoDriver {
    /// Create a new IO driver.
    ///
    /// `event_capacity`: size of the mio events buffer (how many events
    /// per `poll` call). 1024 is typical.
    ///
    /// `token_capacity`: initial token slot count. Grows as needed.
    pub(crate) fn new(event_capacity: usize, token_capacity: usize) -> io::Result<Self> {
        let poll = Poll::new()?;
        let mio_waker = Arc::new(mio::Waker::new(poll.registry(), WAKER_TOKEN)?);
        let events = Events::with_capacity(event_capacity);

        let mut wakers = Vec::with_capacity(token_capacity);
        wakers.resize(token_capacity, None);

        Ok(Self {
            poll,
            events,
            mio_waker,
            wakers,
            free_head: if token_capacity > 0 { 0 } else { NO_FREE },
        })
    }

    /// Returns a clone of the mio waker for breaking out of epoll_wait.
    /// Used by the root future's waker and by task wakers that fire
    /// outside the poll cycle.
    pub(crate) fn mio_waker(&self) -> Arc<mio::Waker> {
        Arc::clone(&self.mio_waker)
    }

    /// Returns a reference to the mio registry for source registration.
    pub(crate) fn registry(&self) -> &mio::Registry {
        self.poll.registry()
    }

    /// Claim a token slot, associating it with a task pointer.
    ///
    /// Returns the `mio::Token` to use when registering a source.
    /// Grows the wakers Vec if no free slots are available.
    pub(crate) fn claim_token(&mut self, task_ptr: *mut u8) -> Token {
        let idx = if self.free_head == NO_FREE {
            // Grow: append a new slot.
            let idx = self.wakers.len();
            self.wakers.push(None);
            idx
        } else {
            // Pop from freelist.
            let idx = self.free_head;
            // Find next free by scanning forward (simple, cold path).
            self.free_head = self.wakers[idx + 1..]
                .iter()
                .position(Option::is_none)
                .map_or(NO_FREE, |pos| idx + 1 + pos);
            idx
        };

        self.wakers[idx] = Some(task_ptr);
        Token(idx)
    }

    /// Release a token slot back to the freelist.
    pub(crate) fn release_token(&mut self, token: Token) {
        let idx = token.0;
        if idx < self.wakers.len() {
            self.wakers[idx] = None;
            // Push to freelist (simple: just update free_head if lower).
            if idx < self.free_head || self.free_head == NO_FREE {
                self.free_head = idx;
            }
        }
    }

    /// Update the task pointer associated with a token.
    ///
    /// Used when a different task takes over an IO source.
    pub(crate) fn set_waker(&mut self, token: Token, task_ptr: *mut u8) {
        if let Some(slot) = self.wakers.get_mut(token.0) {
            *slot = Some(task_ptr);
        }
    }

    /// Poll mio for IO events and wake associated tasks.
    ///
    /// `timeout`: `None` blocks indefinitely, `Some(Duration::ZERO)` is
    /// non-blocking. Returns the number of tasks woken.
    pub(crate) fn poll_io(&mut self, timeout: Option<Duration>) -> io::Result<usize> {
        self.poll.poll(&mut self.events, timeout)?;

        let mut woken = 0;
        for event in &self.events {
            let token = event.token();
            if token == WAKER_TOKEN {
                // Mio waker fired — root future or external wake. Not a task.
                continue;
            }
            let idx = token.0;
            if let Some(Some(task_ptr)) = self.wakers.get(idx) {
                let ptr = *task_ptr;
                // Wake the task: set is_queued flag, push to ready queue.
                // SAFETY: task_ptr points to a live Task in the slab.
                // The waker TLS must be set (we're inside the poll loop).
                unsafe {
                    if !crate::task::is_queued(ptr) {
                        crate::task::set_queued(ptr, true);
                        waker::push_ready(ptr);
                        woken += 1;
                    }
                }
            }
            // Stale tokens (None) are silently skipped — spurious wakeup.
        }

        Ok(woken)
    }
}

// =============================================================================
// IoHandle — Copy handle for tasks
// =============================================================================

/// [`Copy`] handle for IO operations from async tasks.
///
/// Provides source registration with the mio reactor. Obtained from
/// [`RuntimeHandle::io`] or similar.
///
/// # Safety
///
/// The raw pointers are valid for the lifetime of the [`Runtime`].
/// Single-threaded — no concurrent access.
#[derive(Clone, Copy)]
pub struct IoHandle {
    /// Pointer to the mio registry (borrowed from Poll, stable).
    registry: *const mio::Registry,
    /// Pointer to the IoDriver for token management.
    driver: *mut IoDriver,
}

impl IoHandle {
    /// Create a handle from driver references.
    pub(crate) fn new(driver: &mut IoDriver) -> Self {
        Self {
            registry: driver.registry() as *const mio::Registry,
            driver: driver as *mut IoDriver,
        }
    }

    /// Register a mio source with the given interest.
    ///
    /// The `task_ptr` is the task to wake on readiness. Returns the
    /// assigned token for use with `reregister` or `deregister`.
    ///
    /// # Safety
    ///
    /// `task_ptr` must point to a live task in the slab.
    pub unsafe fn register(
        &self,
        source: &mut impl Source,
        interest: Interest,
        task_ptr: *mut u8,
    ) -> io::Result<Token> {
        // SAFETY: driver pointer is valid (Runtime lifetime).
        let driver = unsafe { &mut *self.driver };
        let token = driver.claim_token(task_ptr);
        // SAFETY: registry pointer is valid (borrowed from Poll).
        let registry = unsafe { &*self.registry };
        registry.register(source, token, interest)?;
        Ok(token)
    }

    /// Re-register a source with updated interest or task.
    ///
    /// # Safety
    ///
    /// `task_ptr` must point to a live task in the slab.
    pub unsafe fn reregister(
        &self,
        source: &mut impl Source,
        token: Token,
        interest: Interest,
        task_ptr: *mut u8,
    ) -> io::Result<()> {
        // SAFETY: driver/registry pointers valid (Runtime lifetime).
        let driver = unsafe { &mut *self.driver };
        driver.set_waker(token, task_ptr);
        let registry = unsafe { &*self.registry };
        registry.reregister(source, token, interest)?;
        Ok(())
    }

    /// Deregister a source and release its token.
    pub fn deregister(&self, source: &mut impl Source, token: Token) -> io::Result<()> {
        // SAFETY: driver/registry pointers valid (Runtime lifetime).
        let driver = unsafe { &mut *self.driver };
        let registry = unsafe { &*self.registry };
        registry.deregister(source)?;
        driver.release_token(token);
        Ok(())
    }
}
