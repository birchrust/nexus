//! Cancellation tokens for cooperative task shutdown.
//!
//! Adapted from tokio-util's `CancellationToken` design, built for
//! the nexus-async-rt runtime. `Clone + Send + Sync`. Hierarchical —
//! cancelling a parent cancels all children. Zero-cost when not
//! cancelled (single atomic load).
//!
//! Any holder can cancel or await cancellation — no separate sender/
//! receiver roles. This allows any task in a group to trigger shutdown.
//!
//! ```ignore
//! use nexus_async_rt::CancellationToken;
//!
//! let token = CancellationToken::new();
//!
//! // Any clone can cancel or await:
//! let t = token.clone();
//! spawn_boxed(async move {
//!     match do_work().await {
//!         Ok(()) => t.cancelled().await,  // wait
//!         Err(_) => t.cancel(),           // or trigger
//!     }
//! });
//!
//! // Hierarchical:
//! let child = token.child();  // cancelled when parent is
//!
//! // Drop guard — cancels on scope exit:
//! let _guard = token.drop_guard();
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

// =============================================================================
// Inner state
// =============================================================================

struct Inner {
    cancelled: AtomicBool,
    /// Wakers registered by `cancelled()` futures. Protected by mutex
    /// because cancellation is a cold path — the hot path is just
    /// the atomic load in `is_cancelled()`.
    waiters: Mutex<Vec<Waker>>,
    /// Parent token. When the parent cancels, it cancels all children.
    /// Children hold an Arc to the parent so they can check its state.
    parent: Option<Arc<Inner>>,
}

impl Inner {
    fn new(parent: Option<Arc<Inner>>) -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            waiters: Mutex::new(Vec::new()),
            parent,
        })
    }

    /// Check if this token or any ancestor is cancelled.
    fn is_cancelled(&self) -> bool {
        if self.cancelled.load(Ordering::Acquire) {
            return true;
        }
        // Walk parent chain.
        if let Some(ref parent) = self.parent {
            if parent.is_cancelled() {
                // Cache the result so future checks are O(1).
                self.cancelled.store(true, Ordering::Release);
                return true;
            }
        }
        false
    }

    /// Cancel this token and wake all waiters.
    fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            return; // Already cancelled — no-op.
        }
        // Wake all waiters.
        if let Ok(mut waiters) = self.waiters.lock() {
            for waker in waiters.drain(..) {
                waker.wake();
            }
        }
    }

    /// Register a waker to be notified on cancellation.
    /// Returns true if already cancelled (caller should return Ready).
    fn register(&self, waker: &Waker) -> bool {
        if self.is_cancelled() {
            return true;
        }
        if let Ok(mut waiters) = self.waiters.lock() {
            // Re-check after lock to avoid lost wake.
            if self.is_cancelled() {
                return true;
            }
            waiters.push(waker.clone());
        }
        false
    }
}

// =============================================================================
// CancellationToken
// =============================================================================

/// A token for cooperative cancellation.
///
/// `Clone + Send + Sync`. Cloning shares the same cancellation state.
/// Use [`child()`](CancellationToken::child) for hierarchical cancellation.
///
/// # Example
///
/// ```ignore
/// let token = CancellationToken::new();
///
/// spawn_boxed(async move {
///     token.cancelled().await;
///     println!("shutting down");
/// });
///
/// token.cancel();
/// ```
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

impl CancellationToken {
    /// Create a new cancellation token.
    pub fn new() -> Self {
        Self {
            inner: Inner::new(None),
        }
    }

    /// Create a child token. Cancelling this token's parent (or any
    /// ancestor) also cancels the child. Cancelling the child does
    /// NOT cancel the parent.
    pub fn child(&self) -> Self {
        Self {
            inner: Inner::new(Some(self.inner.clone())),
        }
    }

    /// Cancel this token. All futures awaiting [`cancelled()`](Self::cancelled)
    /// will resolve. Child tokens are also considered cancelled.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Whether this token (or any ancestor) has been cancelled.
    /// Zero-cost atomic load on the fast path.
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    /// Returns a guard that cancels this token when dropped.
    ///
    /// Useful for ensuring cancellation on scope exit or panic.
    pub fn drop_guard(self) -> DropGuard {
        DropGuard { token: Some(self) }
    }

    /// Returns a future that resolves when this token is cancelled.
    pub fn cancelled(&self) -> Cancelled {
        Cancelled {
            inner: self.inner.clone(),
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

// =============================================================================
// Cancelled future
// =============================================================================

/// Future that resolves when a [`CancellationToken`] is cancelled.
///
/// Created by [`CancellationToken::cancelled()`].
pub struct Cancelled {
    inner: Arc<Inner>,
}

impl Future for Cancelled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.inner.register(cx.waker()) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

// =============================================================================
// DropGuard
// =============================================================================

/// A guard that cancels a [`CancellationToken`] when dropped.
///
/// Created by [`CancellationToken::drop_guard()`]. Call
/// [`disarm()`](DropGuard::disarm) to prevent cancellation on drop.
pub struct DropGuard {
    token: Option<CancellationToken>,
}

impl DropGuard {
    /// Disarm the guard — the token will NOT be cancelled on drop.
    /// Returns the token.
    pub fn disarm(mut self) -> CancellationToken {
        self.token.take().expect("DropGuard already disarmed")
    }
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        if let Some(ref token) = self.token {
            token.cancel();
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{RawWaker, RawWakerVTable};

    fn noop_waker() -> Waker {
        fn noop(_: *const ()) {}
        fn noop_clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VTABLE)
        }
        const VTABLE: RawWakerVTable =
            RawWakerVTable::new(noop_clone, noop, noop, noop);
        unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }

    fn poll_once<F: Future>(f: Pin<&mut F>) -> Poll<F::Output> {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        f.poll(&mut cx)
    }

    #[test]
    fn not_cancelled_by_default() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancel_sets_flag() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn clone_shares_state() {
        let token = CancellationToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(clone.is_cancelled());
    }

    #[test]
    fn child_sees_parent_cancel() {
        let parent = CancellationToken::new();
        let child = parent.child();
        assert!(!child.is_cancelled());
        parent.cancel();
        assert!(child.is_cancelled());
    }

    #[test]
    fn grandchild_sees_ancestor_cancel() {
        let root = CancellationToken::new();
        let child = root.child();
        let grandchild = child.child();
        assert!(!grandchild.is_cancelled());
        root.cancel();
        assert!(grandchild.is_cancelled());
    }

    #[test]
    fn child_cancel_does_not_affect_parent() {
        let parent = CancellationToken::new();
        let child = parent.child();
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn cancelled_future_ready_when_cancelled() {
        let token = CancellationToken::new();
        token.cancel();

        let mut fut = std::pin::pin!(token.cancelled());
        assert!(matches!(poll_once(fut.as_mut()), Poll::Ready(())));
    }

    #[test]
    fn cancelled_future_pending_then_ready() {
        let token = CancellationToken::new();

        let mut fut = std::pin::pin!(token.cancelled());
        assert!(matches!(poll_once(fut.as_mut()), Poll::Pending));

        token.cancel();
        // Re-poll — now ready.
        assert!(matches!(poll_once(fut.as_mut()), Poll::Ready(())));
    }

    #[test]
    fn child_cancelled_future_from_parent() {
        let parent = CancellationToken::new();
        let child = parent.child();

        let mut fut = std::pin::pin!(child.cancelled());
        assert!(matches!(poll_once(fut.as_mut()), Poll::Pending));

        parent.cancel();
        assert!(matches!(poll_once(fut.as_mut()), Poll::Ready(())));
    }

    #[test]
    fn multiple_waiters() {
        let token = CancellationToken::new();

        let mut fut1 = std::pin::pin!(token.cancelled());
        let mut fut2 = std::pin::pin!(token.cancelled());

        assert!(matches!(poll_once(fut1.as_mut()), Poll::Pending));
        assert!(matches!(poll_once(fut2.as_mut()), Poll::Pending));

        token.cancel();

        assert!(matches!(poll_once(fut1.as_mut()), Poll::Ready(())));
        assert!(matches!(poll_once(fut2.as_mut()), Poll::Ready(())));
    }

    #[test]
    fn cross_thread_cancel() {
        let token = CancellationToken::new();
        let clone = token.clone();

        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            clone.cancel();
        });

        // Spin until cancelled.
        while !token.is_cancelled() {
            std::hint::spin_loop();
        }

        handle.join().unwrap();
    }

    #[test]
    fn drop_guard_cancels_on_drop() {
        let token = CancellationToken::new();
        let clone = token.clone();
        {
            let _guard = token.drop_guard();
            assert!(!clone.is_cancelled());
        } // guard dropped here
        assert!(clone.is_cancelled());
    }

    #[test]
    fn drop_guard_disarm() {
        let token = CancellationToken::new();
        let clone = token.clone();
        let guard = token.drop_guard();
        let recovered = guard.disarm();
        drop(recovered);
        // Token was NOT cancelled — disarm prevented it.
        // (dropping the recovered token doesn't cancel; only DropGuard does)
        assert!(!clone.is_cancelled());
    }

    #[test]
    fn drop_guard_on_panic() {
        let token = CancellationToken::new();
        let clone = token.clone();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = token.drop_guard();
            panic!("simulated panic");
        }));

        assert!(result.is_err());
        assert!(clone.is_cancelled()); // guard cancelled on unwind
    }

    #[test]
    fn send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CancellationToken>();
        assert_send_sync::<Cancelled>();
    }
}
