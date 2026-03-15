//! Shared and mutable resource references for handler parameters.
//!
//! [`Res<T>`] and [`ResMut<T>`] appear in handler function signatures to
//! declare read and write dependencies on [`World`](crate::World) resources.
//! They are produced by [`Param::fetch`](crate::Param::fetch) during dispatch
//! and deref to the inner `T` transparently.
//!
//! Both carry change-detection metadata: [`is_changed()`](Res::is_changed)
//! compares the resource's `changed_at` stamp against the world's current
//! sequence. [`ResMut`] stamps `changed_at` on [`DerefMut`] — the act of
//! writing is the change signal, no manual marking needed.
//!
//! For optional dependencies, use [`Option<Res<T>>`] or
//! [`Option<ResMut<T>>`] — these resolve to `None` if the type was not
//! registered, rather than panicking at build time.
//!
//! # Examples
//!
//! ```
//! use nexus_rt::{WorldBuilder, Res, ResMut, IntoHandler, Handler};
//!
//! fn process(config: Res<u64>, mut state: ResMut<bool>, _event: ()) {
//!     if *config > 10 {
//!         *state = true; // stamps changed_at
//!     }
//! }
//!
//! let mut builder = WorldBuilder::new();
//! builder.register::<u64>(42);
//! builder.register::<bool>(false);
//! let mut world = builder.build();
//!
//! let mut handler = process.into_handler(world.registry());
//! handler.run(&mut world, ());
//!
//! assert!(*world.resource::<bool>());
//! ```

use std::cell::Cell;
use std::ops::{Deref, DerefMut};

use crate::world::Sequence;

/// Shared reference to a resource in [`World`](crate::World).
///
/// Analogous to Bevy's `Res<T>`.
///
/// Appears in handler function signatures to declare a read dependency.
/// Derefs to the inner value transparently. Carries change-detection
/// metadata — call [`is_changed`](Self::is_changed) to check.
///
/// For exclusive write access, use [`ResMut<T>`]. For optional read
/// access (no panic if unregistered), use [`Option<Res<T>>`].
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
pub struct Res<'w, T: 'static> {
    value: &'w T,
    changed_at: Sequence,
    current_sequence: Sequence,
}

impl<'w, T: 'static> Res<'w, T> {
    pub(crate) fn new(value: &'w T, changed_at: Sequence, current_sequence: Sequence) -> Self {
        Self {
            value,
            changed_at,
            current_sequence,
        }
    }

    /// Returns `true` if the resource was modified during the current sequence.
    pub fn is_changed(&self) -> bool {
        self.changed_at == self.current_sequence
    }

    /// Returns `true` if the resource was modified after `since`.
    ///
    /// Unlike [`is_changed`](Self::is_changed) (equality check against
    /// current sequence), this uses `>` — suitable for checking whether
    /// any event since a prior checkpoint wrote this resource.
    ///
    /// Relies on numeric ordering of the underlying `u64` counter.
    /// Not wrap-safe, but at one increment per event the `u64` sequence
    /// space takes ~584 years at 1 GHz to exhaust.
    pub fn changed_after(&self, since: Sequence) -> bool {
        self.changed_at.0 > since.0
    }
}

impl<T: std::fmt::Debug + 'static> std::fmt::Debug for Res<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<T: 'static> Deref for Res<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.value
    }
}

/// Mutable reference to a resource in [`World`](crate::World).
///
/// Analogous to Bevy's `ResMut<T>`.
///
/// Appears in handler function signatures to declare a write dependency.
/// Derefs to the inner value transparently. Stamps the resource's
/// `changed_at` sequence on [`DerefMut`] — the act of writing is the
/// change signal.
///
/// For shared read access, use [`Res<T>`]. For optional write access
/// (no panic if unregistered), use [`Option<ResMut<T>>`].
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
pub struct ResMut<'w, T: 'static> {
    value: &'w mut T,
    changed_at: &'w Cell<Sequence>,
    current_sequence: Sequence,
}

impl<'w, T: 'static> ResMut<'w, T> {
    pub(crate) fn new(
        value: &'w mut T,
        changed_at: &'w Cell<Sequence>,
        current_sequence: Sequence,
    ) -> Self {
        Self {
            value,
            changed_at,
            current_sequence,
        }
    }

    /// Returns `true` if the resource was modified during the current sequence.
    pub fn is_changed(&self) -> bool {
        self.changed_at.get() == self.current_sequence
    }

    /// Returns `true` if the resource was modified after `since`.
    ///
    /// Unlike [`is_changed`](Self::is_changed) (equality check against
    /// current sequence), this uses `>` — suitable for checking whether
    /// any event since a prior checkpoint wrote this resource.
    ///
    /// Relies on numeric ordering of the underlying `u64` counter.
    /// Not wrap-safe, but at one increment per event the `u64` sequence
    /// space takes ~584 years at 1 GHz to exhaust.
    pub fn changed_after(&self, since: Sequence) -> bool {
        self.changed_at.get().0 > since.0
    }
}

impl<T: std::fmt::Debug + 'static> std::fmt::Debug for ResMut<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<T: 'static> Deref for ResMut<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: 'static> DerefMut for ResMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        self.changed_at.set(self.current_sequence);
        self.value
    }
}

// =============================================================================
// Seq / SeqMut — sequence number access
// =============================================================================

/// Read-only access to the world's current sequence number.
///
/// Appears in handler function signatures alongside other params.
/// Derefs to [`Sequence`].
///
/// # Example
///
/// ```ignore
/// use nexus_rt::{Seq, Handler, IntoHandler};
///
/// fn log_event(seq: Seq, event: u64) {
///     println!("seq={:?}, event={}", seq.get(), event);
/// }
/// ```
pub struct Seq(pub(crate) Sequence);

impl Seq {
    /// Returns the sequence value.
    pub fn get(&self) -> Sequence {
        self.0
    }
}

impl Deref for Seq {
    type Target = Sequence;

    #[inline(always)]
    fn deref(&self) -> &Sequence {
        &self.0
    }
}

impl std::fmt::Debug for Seq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Mutable access to the world's sequence number.
///
/// Allows handlers to advance the sequence for stamping outbound messages.
/// Each call to [`advance`](Self::advance) returns a new monotonically increasing
/// sequence number.
///
/// # Example
///
/// ```ignore
/// use nexus_rt::{SeqMut, Handler, IntoHandler};
///
/// fn on_trade(mut seq: SeqMut, event: Trade) {
///     let seq_a = seq.advance();
///     publish(Message { seq: seq_a, .. });
///
///     let seq_b = seq.advance();
///     publish(Message { seq: seq_b, .. });
/// }
/// ```
pub struct SeqMut<'w>(pub(crate) &'w Cell<Sequence>);

impl SeqMut<'_> {
    /// Returns the current sequence value.
    pub fn get(&self) -> Sequence {
        self.0.get()
    }

    /// Advance to the next sequence number and return it.
    pub fn advance(&mut self) -> Sequence {
        let next = Sequence(self.0.get().0.wrapping_add(1));
        self.0.set(next);
        next
    }
}

impl std::fmt::Debug for SeqMut<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn res_deref() {
        let val = 42u64;
        let res = Res::new(&val, Sequence::default(), Sequence::default());
        assert_eq!(*res, 42);
    }

    #[test]
    fn res_is_changed() {
        let val = 42u64;
        let tick = Sequence::default();
        let res = Res::new(&val, tick, tick);
        assert!(res.is_changed());
    }

    #[test]
    fn res_not_changed() {
        let val = 42u64;
        // changed_at=0, current_sequence=1 → not changed
        let res = Res::new(&val, Sequence::default(), Sequence(1));
        assert!(!res.is_changed());
    }

    #[test]
    fn res_mut_deref_mut() {
        let mut val = 1u64;
        let changed_at = Cell::new(Sequence::default());
        let mut res = ResMut::new(&mut val, &changed_at, Sequence::default());
        *res = 99;
        assert_eq!(*res, 99);
        // Intentional drop to end mutable borrow before asserting.
        #[allow(clippy::drop_non_drop)]
        drop(res);
        assert_eq!(val, 99);
    }

    #[test]
    fn res_mut_deref_mut_stamps() {
        let mut val = 1u64;
        let changed_at = Cell::new(Sequence(0));
        let current = Sequence(5);
        let mut res = ResMut::new(&mut val, &changed_at, current);

        // Before DerefMut — changed_at is still 0
        assert_eq!(changed_at.get(), Sequence(0));

        *res = 99;

        // After DerefMut — changed_at stamped to current_sequence
        assert_eq!(changed_at.get(), Sequence(5));
    }

    #[test]
    fn res_mut_deref_does_not_stamp() {
        let mut val = 42u64;
        let changed_at = Cell::new(Sequence(0));
        let current = Sequence(5);
        let res = ResMut::new(&mut val, &changed_at, current);

        // Deref (shared) — read only, should not stamp
        let _ = *res;
        assert_eq!(changed_at.get(), Sequence(0));
    }

    #[test]
    fn res_changed_after() {
        let val = 42u64;
        // changed_at=3, since=1 → changed after
        let res = Res::new(&val, Sequence(3), Sequence(5));
        assert!(res.changed_after(Sequence(1)));
    }

    #[test]
    fn res_changed_after_equal_is_false() {
        let val = 42u64;
        // changed_at=3, since=3 → NOT changed after (equal, not greater)
        let res = Res::new(&val, Sequence(3), Sequence(5));
        assert!(!res.changed_after(Sequence(3)));
    }

    #[test]
    fn res_mut_changed_after() {
        let mut val = 1u64;
        let changed_at = Cell::new(Sequence(5));
        let res = ResMut::new(&mut val, &changed_at, Sequence(5));
        assert!(res.changed_after(Sequence(2)));
        assert!(!res.changed_after(Sequence(5)));
        assert!(!res.changed_after(Sequence(7)));
    }

    #[test]
    fn res_mut_is_changed() {
        let mut val = 1u64;
        let changed_at = Cell::new(Sequence(3));
        let res = ResMut::new(&mut val, &changed_at, Sequence(3));
        assert!(res.is_changed());

        let res2 = ResMut::new(&mut val, &changed_at, Sequence(4));
        assert!(!res2.is_changed());
    }
}
