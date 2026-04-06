//! Tokio compatibility layer.
//!
//! Allows polling tokio-based futures from the nexus-async-rt executor.
//! Tokio's background reactor watches file descriptors; our executor
//! owns and polls the futures. Cross-thread wakers bridge the gap.
//!
//! A lazy tokio runtime (single worker thread) is created on first use.
//! Its only job is running the IO reactor (epoll) — it never schedules
//! or polls futures.
//!
//! # How it works
//!
//! 1. `with_tokio(|| future_expr)` installs tokio's runtime context on
//!    the current thread via `Handle::enter()`. The closure runs with
//!    tokio context available so tokio types can be constructed.
//! 2. When polled, the tokio future registers its fds with tokio's
//!    reactor and stores a waker.
//! 3. That waker is our cross-thread waker — it pushes to the
//!    intrusive inbox + conditionally pokes the eventfd.
//! 4. When tokio's reactor detects IO readiness, it fires our waker.
//! 5. Our executor wakes up, re-polls the task, the future reads data.
//!
//! Tokio never polls the future. It just fires wakers.
//!
//! # Performance
//!
//! The waker bridge adds ~76ns per waker hop (measured with tokio
//! oneshot channel, pinned to separate physical cores):
//!
//! | Percentile | Busy spin | Park mode |
//! |-----------|-----------|-----------|
//! | p50       | 76 ns     | 75 ns     |
//! | p90       | 89 ns     | 92 ns     |
//! | p99       | 110 ns    | 117 ns    |
//! | p99.9     | 299 ns    | 2.0 µs   |
//!
//! TCP echo loopback (write + read, two bridge hops): ~8µs p50.
//!
//! # Usage
//!
//! ```ignore
//! use nexus_async_rt::tokio_compat::with_tokio;
//!
//! rt.block_on(async {
//!     // Single operation:
//!     let stream = with_tokio(|| TcpStream::connect(addr)).await?;
//!
//!     // Multiple awaits in one block:
//!     let data = with_tokio(|| async {
//!         let mut stream = TcpStream::connect(addr).await?;
//!         stream.write_all(b"hello").await?;
//!         let mut buf = [0u8; 64];
//!         let n = stream.read(&mut buf).await?;
//!         Ok::<_, io::Error>(buf[..n].to_vec())
//!     }).await?;
//!
//!     // Tokio ecosystem crates (e.g., databento):
//!     let client = with_tokio(|| databento::LiveClient::connect(key)).await?;
//!     loop {
//!         let record = with_tokio(|| client.next_record()).await?;
//!         process(record);  // runs on our executor, no wrapper needed
//!     }
//! });
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll, Waker};

/// Global lazy tokio runtime. Single worker thread for the IO reactor.
static TOKIO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn tokio_runtime() -> &'static tokio::runtime::Runtime {
    TOKIO_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("failed to create tokio compatibility runtime")
    })
}

// Thread-local flag: tokio context installed on this thread.
// Set once, shared across all `with_tokio` calls. Avoids the
// "guards dropped out of order" panic from nested EnterGuards.
thread_local! {
    static TOKIO_ENTERED: Cell<bool> = const { Cell::new(false) };
}

use std::cell::Cell;

fn ensure_tokio_context() {
    TOKIO_ENTERED.with(|entered| {
        if !entered.get() {
            // Leak the guard — it lives for the thread's lifetime.
            // This is fine: the tokio runtime is 'static, and the
            // guard just sets TLS on this thread.
            std::mem::forget(tokio_runtime().enter());
            entered.set(true);
        }
    });
}

/// Run a closure with tokio context installed, returning a future
/// that can be polled from nexus-async-rt.
///
/// The closure runs immediately with tokio's runtime context available,
/// so tokio types can be constructed (e.g., `tokio::time::sleep()`).
/// The returned future is then polled by our executor with cross-thread
/// wakers bridging tokio's reactor back to us.
///
/// # Panics
///
/// Panics if called outside [`Runtime::block_on`](crate::Runtime::block_on).
pub fn with_tokio<F, Fut>(f: F) -> TokioCompat<Fut>
where
    F: FnOnce() -> Fut,
    Fut: Future,
{
    ensure_tokio_context();
    let future = f();
    TokioCompat { future }
}

/// Future wrapper that polls an inner future with tokio context installed.
///
/// Created by [`with_tokio()`].
pub struct TokioCompat<F> {
    future: F,
}

impl<F: Future> Future for TokioCompat<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: we only project to `future` (structurally pinned).
        let this = unsafe { self.get_unchecked_mut() };

        // Build a cross-thread waker for this task.
        let cross_waker = make_cross_waker(cx);
        let mut cross_cx = Context::from_waker(&cross_waker);

        // Poll the inner future with cross-thread waker.
        // Tokio context installed via TLS (ensure_tokio_context).
        let future = unsafe { Pin::new_unchecked(&mut this.future) };
        future.poll(&mut cross_cx)
    }
}

/// Build a cross-thread waker from the current context.
///
/// If the waker is our local runtime waker, extract the task pointer
/// and build a cross-thread waker. If it's already cross-thread safe
/// (e.g., root future waker), clone it directly.
fn make_cross_waker(cx: &Context<'_>) -> Waker {
    crate::waker::task_ptr_from_local_waker(cx.waker()).map_or_else(
        || cx.waker().clone(),
        |task_ptr| {
            let ctx = crate::cross_wake::cross_wake_context()
                .expect("with_tokio() requires runtime context");
            CrossTaskWaker::into_waker(task_ptr, ctx)
        },
    )
}

/// Cross-thread waker that pushes to our intrusive inbox.
/// Created per-poll of `TokioCompat`. Shared across tokio's reactor
/// via Arc clone.
struct CrossTaskWaker {
    task_ptr: *mut u8,
    ctx: std::sync::Arc<crate::cross_wake::CrossWakeContext>,
}

// SAFETY: task_ptr is only used for atomic operations (try_set_queued,
// is_completed) and queue push — all thread-safe.
unsafe impl Send for CrossTaskWaker {}
unsafe impl Sync for CrossTaskWaker {}

impl CrossTaskWaker {
    fn into_waker(
        task_ptr: *mut u8,
        ctx: std::sync::Arc<crate::cross_wake::CrossWakeContext>,
    ) -> Waker {
        // Increment task refcount — the waker holds a reference.
        unsafe { crate::task::ref_inc(task_ptr) };
        let arc = std::sync::Arc::new(Self { task_ptr, ctx });
        Waker::from(arc)
    }
}

impl std::task::Wake for CrossTaskWaker {
    fn wake(self: std::sync::Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &std::sync::Arc<Self>) {
        unsafe {
            crate::cross_wake::wake_task_cross_thread(self.task_ptr, &self.ctx);
        }
    }
}

impl Drop for CrossTaskWaker {
    fn drop(&mut self) {
        let should_free = unsafe { crate::task::ref_dec(self.task_ptr) };
        if should_free {
            // Task completed + all wakers dropped. The executor will
            // clean up the slot on its next poll cycle.
        }
    }
}
