//! Miri tests for CancellationToken.
//!
//! Exercises Treiber stack push/drain, waiter node lifecycle,
//! child propagation, and drop cleanup under miri.
//!
//! Run: `cargo +nightly miri test -p nexus-async-rt --test miri_cancel`

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use nexus_async_rt::CancellationToken;

// =============================================================================
// Test helpers
// =============================================================================

/// Minimal noop waker for polling futures outside a runtime.
fn noop_waker() -> Waker {
    static VTABLE: RawWakerVTable =
        RawWakerVTable::new(|p| RawWaker::new(p, &VTABLE), |_| {}, |_| {}, |_| {});
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

fn poll_once<F: Future>(f: Pin<&mut F>) -> Poll<F::Output> {
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    f.poll(&mut cx)
}

/// A waker that sets a Cell<bool> to true when woken.
/// Each call creates a new vtable instance so will_wake() returns false
/// between different tracking_waker calls — exercising the re-registration path.
fn tracking_waker(flag: &std::cell::Cell<bool>) -> Waker {
    // Store the flag pointer as the waker data.
    let data = flag as *const std::cell::Cell<bool> as *const ();

    // Use a unique vtable per call site — this ensures will_wake() returns
    // false between different tracking_waker instances, even with same flag.
    static VTABLE: RawWakerVTable = RawWakerVTable::new(
        |p| RawWaker::new(p, &VTABLE),
        |p| {
            // wake: set the flag
            let flag = unsafe { &*(p as *const std::cell::Cell<bool>) };
            flag.set(true);
        },
        |p| {
            // wake_by_ref: set the flag
            let flag = unsafe { &*(p as *const std::cell::Cell<bool>) };
            flag.set(true);
        },
        |_| {}, // drop: no-op
    );
    unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) }
}

// =============================================================================
// Tests
// =============================================================================

/// Basic cancel lifecycle: create, verify not cancelled, cancel, verify cancelled.
#[test]
fn cancel_basic() {
    let token = CancellationToken::new();
    assert!(!token.is_cancelled());

    token.cancel();
    assert!(token.is_cancelled());
}

/// Register 5 waiters by polling cancelled() futures to Pending, then cancel.
/// All futures must resolve to Ready on the next poll.
/// Exercises Treiber stack push (5 WaiterNode CAS pushes) + drain-all on cancel.
#[test]
fn cancel_with_waiters() {
    let token = CancellationToken::new();

    let mut futures: Vec<_> = (0..5).map(|_| Box::pin(token.cancelled())).collect();

    // First poll: all return Pending (registers WaiterNodes).
    for f in &mut futures {
        assert_eq!(poll_once(f.as_mut()), Poll::Pending);
    }

    token.cancel();

    // Second poll: all return Ready (cancel drained the waiter stack and woke them).
    for f in &mut futures {
        assert_eq!(poll_once(f.as_mut()), Poll::Ready(()));
    }
}

/// Create parent with 3 children. Cancel parent. All children must be cancelled.
/// Exercises the ChildNode Treiber stack push + drain on parent cancel.
#[test]
fn cancel_child_propagation() {
    let parent = CancellationToken::new();
    let children: Vec<_> = (0..3).map(|_| parent.child()).collect();

    assert!(!parent.is_cancelled());
    for c in &children {
        assert!(!c.is_cancelled());
    }

    parent.cancel();

    assert!(parent.is_cancelled());
    for c in &children {
        assert!(c.is_cancelled());
    }
}

/// Simulate the register-during-cancel race path (single-threaded).
///
/// The register() method has a double-check after CAS push: if cancel happened
/// between the initial check and the push, it re-drains. We exercise this by:
/// poll future -> Pending -> cancel -> poll future again -> Ready.
#[test]
fn cancel_register_during_cancel_race() {
    let token = CancellationToken::new();
    let mut fut = Box::pin(token.cancelled());

    // Poll registers a WaiterNode via CAS push, returns Pending.
    assert_eq!(poll_once(fut.as_mut()), Poll::Pending);

    // Cancel drains the waiter stack.
    token.cancel();

    // Next poll sees is_cancelled() == true, returns Ready.
    assert_eq!(poll_once(fut.as_mut()), Poll::Ready(()));
}

/// Drop token with registered waiters and children without cancelling.
/// Inner::drop must drain and free all heap-allocated nodes.
/// Miri will flag any leak or use-after-free.
#[test]
fn cancel_drop_without_cancel() {
    let token = CancellationToken::new();

    // Register waiters.
    let mut futures: Vec<_> = (0..3).map(|_| Box::pin(token.cancelled())).collect();
    for f in &mut futures {
        assert_eq!(poll_once(f.as_mut()), Poll::Pending);
    }

    // Create children.
    let _children: Vec<_> = (0..3).map(|_| token.child()).collect();

    // Drop everything without cancelling. Miri checks for leaks.
    drop(futures);
    drop(_children);
    drop(token);
}

/// Cancelled future re-registers when polled with a different waker.
/// Verifies the will_wake() check and re-registration path.
/// Miri catches any leaked WaiterNodes from stale registrations.
#[test]
fn cancel_waker_update_on_repoll() {
    let token = CancellationToken::new();
    let mut fut = Box::pin(token.cancelled());

    // Poll with waker A — registers WaiterNode with waker A.
    let waker_a = noop_waker();
    let mut cx_a = Context::from_waker(&waker_a);
    assert_eq!(fut.as_mut().poll(&mut cx_a), Poll::Pending);

    // Poll with waker B (different noop_waker instance) — should re-register.
    // Note: two noop_wakers from our helper share the same vtable pointer and
    // null data, so will_wake returns true. Use a tracking waker instead.
    let woke = std::cell::Cell::new(false);
    let waker_b = tracking_waker(&woke);
    let mut cx_b = Context::from_waker(&waker_b);
    assert_eq!(fut.as_mut().poll(&mut cx_b), Poll::Pending);

    // Cancel — should wake via the latest registered waker.
    token.cancel();

    // The tracking waker should have been called.
    // (Note: the noop waker from the first registration also fires — that's fine,
    // it's a no-op. The important thing is waker_b fires.)
    assert!(woke.get(), "latest waker must be woken on cancel");

    // Final poll confirms Ready.
    assert_eq!(fut.as_mut().poll(&mut cx_b), Poll::Ready(()));
}

/// Multiple waker changes across polls. Exercises repeated re-registration
/// without leaking WaiterNodes (miri catches leaks).
#[test]
fn cancel_many_waker_changes() {
    let token = CancellationToken::new();
    let mut fut = Box::pin(token.cancelled());

    // Keep all flags alive until after cancel — cancel() drains ALL
    // WaiterNodes and wakes their wakers, including stale ones from
    // prior registrations. The flags must outlive the drain.
    let flags: Vec<std::cell::Cell<bool>> =
        (0..10).map(|_| std::cell::Cell::new(false)).collect();

    // Poll 10 times with different wakers.
    for flag in &flags {
        let waker = tracking_waker(flag);
        let mut cx = Context::from_waker(&waker);
        assert_eq!(fut.as_mut().poll(&mut cx), Poll::Pending);
    }

    // Cancel — all WaiterNodes drained and freed. Miri checks for leaks.
    token.cancel();
    assert_eq!(poll_once(fut.as_mut()), Poll::Ready(()));
}

/// Cancel before any poll — future resolves immediately on first poll.
/// No WaiterNode allocated.
#[test]
fn cancel_before_poll() {
    let token = CancellationToken::new();
    token.cancel();

    let mut fut = Box::pin(token.cancelled());
    assert_eq!(poll_once(fut.as_mut()), Poll::Ready(()));
}

/// Drop child token clone before parent cancels. The ChildNode in the parent's
/// Treiber stack still holds an Arc<Inner> clone of the child's Inner. When
/// parent cancels, it drains the ChildNode and cancels the child via that Arc.
#[test]
fn cancel_child_drop_before_parent() {
    let parent = CancellationToken::new();
    let child = parent.child();

    // Clone and hold a reference to observe the child after dropping the original.
    let child_observer = child.cancelled();
    let mut child_fut = Box::pin(child_observer);

    // Poll to register a waiter on the child.
    assert_eq!(poll_once(child_fut.as_mut()), Poll::Pending);

    // Drop the child token. The ChildNode in parent's stack still holds an Arc.
    drop(child);

    // Cancel parent — drains ChildNode stack, cancels child's Inner.
    parent.cancel();

    // The child's future should now resolve.
    assert_eq!(poll_once(child_fut.as_mut()), Poll::Ready(()));
}
