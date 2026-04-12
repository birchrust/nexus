//! Cancellation tokens for cooperative task shutdown.
//!
//! Adapted from tokio-util's `CancellationToken` design, built for
//! the nexus-async-rt runtime. `Clone + Send + Sync`. Hierarchical —
//! cancelling a parent cancels all children.
//!
//! Lock-free: `is_cancelled()` is a single atomic load. Registration
//! and cancellation use atomic Treiber stacks (CAS on head). No mutex.
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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::task::{Context, Poll, Waker};

// =============================================================================
// Inner state — lock-free via atomic Treiber stacks
// =============================================================================

struct Inner {
    cancelled: AtomicBool,
    /// Head of the waiter Treiber stack. Each node is a heap-allocated
    /// `WaiterNode`. Push via CAS, drain-all via swap-to-null on cancel.
    waiter_head: AtomicPtr<WaiterNode>,
    /// Head of the child Treiber stack. Each node is a heap-allocated
    /// `ChildNode`. Same push/drain pattern.
    child_head: AtomicPtr<ChildNode>,
}

struct WaiterNode {
    waker: Waker,
    next: *mut WaiterNode,
}

struct ChildNode {
    inner: Arc<Inner>,
    next: *mut ChildNode,
}

// SAFETY: WaiterNode/ChildNode are only accessed via atomic stack
// operations (push from any thread, drain from cancelling thread).
// The Waker inside is Send+Sync. Arc<Inner> is Send+Sync.
unsafe impl Send for WaiterNode {}
unsafe impl Send for ChildNode {}

impl Inner {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            waiter_head: AtomicPtr::new(std::ptr::null_mut()),
            child_head: AtomicPtr::new(std::ptr::null_mut()),
        })
    }

    /// O(1) — single atomic load.
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Cancel: set flag, drain and wake all waiters, drain and cancel all children.
    ///
    /// Idempotent — safe to call multiple times. The flag swap is a no-op
    /// if already true. The list drains are also idempotent (swap to null
    /// on an already-null list is a no-op). This is important because
    /// register()/add_child() call cancel() to catch nodes pushed during
    /// a race window.
    fn cancel(&self) {
        // Set the flag. If it was already set, we still drain below to
        // catch nodes pushed between a prior cancel()'s drain and now.
        self.cancelled.store(true, Ordering::Release);

        // Drain waiters — swap head to null, walk the list.
        let mut waiter = self
            .waiter_head
            .swap(std::ptr::null_mut(), Ordering::AcqRel);
        while !waiter.is_null() {
            // SAFETY: node was allocated by register() via Box::into_raw.
            let node = unsafe { Box::from_raw(waiter) };
            waiter = node.next;
            node.waker.wake();
        }

        // Drain children — swap head to null, cancel each.
        let mut child = self.child_head.swap(std::ptr::null_mut(), Ordering::AcqRel);
        while !child.is_null() {
            let node = unsafe { Box::from_raw(child) };
            child = node.next;
            node.inner.cancel();
        }
    }

    /// Register a child. If already cancelled, cancels the child immediately.
    fn add_child(&self, child: &Arc<Inner>) {
        let node = Box::into_raw(Box::new(ChildNode {
            inner: child.clone(),
            next: std::ptr::null_mut(),
        }));

        // CAS push onto the child stack.
        loop {
            // Check cancelled before pushing — avoid leaking the node.
            if self.is_cancelled() {
                // SAFETY: we just allocated this node.
                let node = unsafe { Box::from_raw(node) };
                node.inner.cancel();
                return;
            }

            let head = self.child_head.load(Ordering::Acquire);
            unsafe { (*node).next = head };
            if self
                .child_head
                .compare_exchange_weak(head, node, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                // Successfully pushed. But check if cancelled between
                // our load and the CAS — if so, the cancel() call may
                // have already drained and missed our node.
                if self.is_cancelled() {
                    // Re-cancel to catch our node (idempotent).
                    self.cancel();
                }
                return;
            }
        }
    }

    /// Register a waker. Returns true if already cancelled.
    fn register(&self, waker: &Waker) -> bool {
        if self.is_cancelled() {
            return true;
        }

        let node = Box::into_raw(Box::new(WaiterNode {
            waker: waker.clone(),
            next: std::ptr::null_mut(),
        }));

        // CAS push onto the waiter stack.
        loop {
            if self.is_cancelled() {
                // SAFETY: we just allocated this node.
                unsafe { drop(Box::from_raw(node)) };
                return true;
            }

            let head = self.waiter_head.load(Ordering::Acquire);
            unsafe { (*node).next = head };
            if self
                .waiter_head
                .compare_exchange_weak(head, node, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                // Check for race: cancelled between load and CAS.
                if self.is_cancelled() {
                    self.cancel(); // idempotent — drains our node
                    return true;
                }
                return false;
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Clean up any remaining nodes (shouldn't happen normally,
        // but guards against leaks if tokens are dropped without cancel).
        let mut waiter = *self.waiter_head.get_mut();
        while !waiter.is_null() {
            let node = unsafe { Box::from_raw(waiter) };
            waiter = node.next;
        }
        let mut child = *self.child_head.get_mut();
        while !child.is_null() {
            let node = unsafe { Box::from_raw(child) };
            child = node.next;
        }
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
            inner: Inner::new(),
        }
    }

    /// Create a child token. Cancelling this token (or any ancestor)
    /// also cancels the child and wakes its waiters. Cancelling the
    /// child does NOT cancel the parent.
    pub fn child(&self) -> Self {
        let child = Self {
            inner: Inner::new(),
        };
        self.inner.add_child(&child.inner);
        child
    }

    /// Cancel this token. All futures awaiting [`cancelled()`](Self::cancelled)
    /// will resolve. Child tokens are also cancelled.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Whether this token has been cancelled.
    /// O(1) — single atomic load. Parent cancellation propagates
    /// eagerly (sets the child's flag), so no chain traversal needed.
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
            registered: false,
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
    registered: bool,
}

impl Future for Cancelled {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.inner.is_cancelled() {
            return Poll::Ready(());
        }
        if !self.registered {
            if self.inner.register(cx.waker()) {
                return Poll::Ready(());
            }
            self.registered = true;
        }
        Poll::Pending
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
        const VTABLE: RawWakerVTable = RawWakerVTable::new(noop_clone, noop, noop, noop);
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
        }
        assert!(clone.is_cancelled());
    }

    #[test]
    fn drop_guard_disarm() {
        let token = CancellationToken::new();
        let clone = token.clone();
        let guard = token.drop_guard();
        let recovered = guard.disarm();
        drop(recovered);
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
        assert!(clone.is_cancelled());
    }

    #[test]
    fn send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CancellationToken>();
        assert_send_sync::<Cancelled>();
    }

    #[test]
    fn drop_without_cancel_cleans_up() {
        // Tokens dropped without cancellation — nodes should be freed.
        let token = CancellationToken::new();
        let _child = token.child();
        let mut fut = std::pin::pin!(token.cancelled());
        let _ = poll_once(fut.as_mut()); // register a waiter
        // Everything dropped — no leak (tested under miri if available).
    }

    #[test]
    fn many_children() {
        let parent = CancellationToken::new();
        let children: Vec<_> = (0..100).map(|_| parent.child()).collect();

        parent.cancel();
        for child in &children {
            assert!(child.is_cancelled());
        }
    }

    #[test]
    fn child_created_after_parent_cancelled() {
        let parent = CancellationToken::new();
        parent.cancel();
        let child = parent.child();
        assert!(child.is_cancelled());
    }
}
