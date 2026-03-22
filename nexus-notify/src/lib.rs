//! Cross-thread notification with conflation and FIFO delivery.
//!
//! [`event_queue()`] creates a `(Notifier, Poller)` pair for signaling which
//! items are ready for processing. An IO thread writes data into shared
//! storage (e.g., a conflation slot) and calls [`Notifier::notify`]. The
//! main event loop calls [`Poller::poll`] or [`Poller::poll_limit`] to
//! discover which tokens fired.
//!
//! # Architecture
//!
//! Two concerns, cleanly separated:
//!
//! - **Dedup flags** — one [`AtomicBool`](core::sync::atomic::AtomicBool)
//!   per token. If the flag is already `true` when a producer calls
//!   [`notify()`](Notifier::notify), the notification is a no-op (conflated).
//!   Single atomic swap, no queue interaction.
//!
//! - **Delivery queue** — a [`nexus_queue`] MPSC ring buffer. When a
//!   producer wins the flag swap (`false → true`), it pushes the token
//!   index into the FIFO queue. The consumer pops and clears the flag,
//!   re-arming it for future notifications.
//!
//! Hot path pointers are cached locally on both [`Notifier`] and
//! [`Poller`] — no `Arc` dereference per operation.
//!
//! # Operations
//!
//! Only three operations matter:
//!
//! - **[`notify(token)`](Notifier::notify)** — signal readiness. Conflated
//!   if already flagged. Returns `Result` so the caller owns the error
//!   policy (`.unwrap()` to crash, `.ok()` to swallow, or match to log).
//!
//! - **[`poll(events)`](Poller::poll)** — drain all ready tokens into the
//!   events buffer.
//!
//! - **[`poll_limit(events, limit)`](Poller::poll_limit)** — drain up to
//!   `limit` tokens. Remaining items stay in the queue for the next call.
//!   Oldest notifications drain first (FIFO) — no starvation under budget.
//!
//! # Invariants
//!
//! - **Flag = `true` ⟺ token is in the queue (or being pushed).** The
//!   consumer clears the flag on pop. Producers only set flags.
//!
//! - **At most one queue entry per token.** The flag gates admission.
//!   Two producers racing on the same token: one wins (pushes), the other
//!   sees `true` (conflated). Never two entries for the same index.
//!
//! - **Queue cannot overflow.** The flag ensures at most one entry per
//!   token. Queue capacity = max tokens. Overflow is an invariant
//!   violation (logic bug), reported via [`NotifyError`].
//!
//! - **FIFO delivery.** The MPSC ring buffer preserves push order. The
//!   consumer sees tokens in the order they were first notified.
//!
//! # Spurious Wakeups
//!
//! If a slab key is freed and reassigned to a new item, a [`notify()`](Notifier::notify)
//! in-flight for the old item fires the token for the new item. The
//! consumer must tolerate spurious wakeups during the transition.
//!
//! The user's responsibilities:
//! 1. Stop calling `notify()` for a token before its key is reused.
//! 2. A callback's token cannot change without informing the producer.
//! 3. Tolerate spurious wakeups during the deregister window.
//!
//! Same contract as mio.
//!
//! # Memory Ordering
//!
//! Flags use `Relaxed` ordering. The flag is a pure dedup gate — it
//! carries no data. The MPSC queue provides the happens-before chain
//! between producer and consumer via its own `Acquire`/`Release` on
//! turn counters. `swap` is a read-modify-write operation that
//! participates in the modification order of its atomic location
//! regardless of the ordering parameter.
//!
//! # Performance (p50 cycles, measured)
//!
//! | Operation | Cycles | Notes |
//! |-----------|--------|-------|
//! | notify (conflated) | 16 | flag swap only |
//! | notify (new) | 16 | flag swap + CAS push |
//! | poll empty | 2 | single failed pop |
//! | poll N=8 | 48 | |
//! | poll N=128 | 684 | ~5.3 cy/token |
//! | poll_limit=32 (4096 ready) | 162 | O(limit) |
//! | cross-thread roundtrip | 362 | ~100ns @ 3.5GHz |
//!
//! # Memory
//!
//! For `max_tokens = 4096`: flags = 4 KB, MPSC queue = 64 KB (rounded
//! to power-of-two), total ~68 KB.
//!
//! # Example
//!
//! ```
//! use nexus_notify::{event_queue, Token};
//!
//! // Setup
//! let (notifier, poller) = event_queue(64);
//! let mut events = nexus_notify::Events::with_capacity(64);
//!
//! // Producer: signal readiness
//! let token = Token::new(0);
//! notifier.notify(token).unwrap();
//!
//! // Consumer: discover what's ready
//! poller.poll(&mut events);
//! assert_eq!(events.len(), 1);
//! assert_eq!(events.as_slice()[0].index(), 0);
//! ```
//!
//! ## With poll_limit (budgeted drain)
//!
//! ```
//! use nexus_notify::{event_queue, Token};
//!
//! let (notifier, poller) = event_queue(256);
//! let mut events = nexus_notify::Events::with_capacity(256);
//!
//! // Many tokens ready
//! for i in 0..100 {
//!     notifier.notify(Token::new(i)).unwrap();
//! }
//!
//! // Drain only 10 per iteration (oldest first)
//! poller.poll_limit(&mut events, 10);
//! assert_eq!(events.len(), 10);
//! assert_eq!(events.as_slice()[0].index(), 0);  // FIFO: oldest first
//!
//! // Remaining 90 stay in the queue
//! poller.poll(&mut events);
//! assert_eq!(events.len(), 90);
//! ```

mod event_queue;

pub use event_queue::{Events, Notifier, NotifyError, Poller, Token, event_queue};
