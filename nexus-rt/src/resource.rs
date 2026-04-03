//! Shared and mutable resource references for handler parameters.
//!
//! [`Res<T>`] and [`ResMut<T>`] appear in handler function signatures to
//! declare read and write dependencies on [`World`](crate::World) resources.
//! They are produced by [`Param::fetch`](crate::Param::fetch) during dispatch
//! and deref to the inner `T` transparently.
//!
//! For optional dependencies, use [`Option<Res<T>>`] or
//! [`Option<ResMut<T>>`] — these resolve to `None` if the type was not
//! registered, rather than panicking at build time.
//!
//! # Examples
//!
//! ```
//! use nexus_rt::{WorldBuilder, Res, ResMut, IntoHandler, Handler, Resource};
//!
//! #[derive(Resource)]
//! struct Config(u64);
//! #[derive(Resource)]
//! struct Flag(bool);
//!
//! fn process(config: Res<Config>, mut state: ResMut<Flag>, _event: ()) {
//!     if config.0 > 10 {
//!         state.0 = true;
//!     }
//! }
//!
//! let mut builder = WorldBuilder::new();
//! builder.register(Config(42));
//! builder.register(Flag(false));
//! let mut world = builder.build();
//!
//! let mut handler = process.into_handler(world.registry());
//! handler.run(&mut world, ());
//!
//! assert!(world.resource::<Flag>().0);
//! ```

use std::cell::Cell;
use std::ops::{Deref, DerefMut};

use crate::Resource;
use crate::world::Sequence;

/// Shared reference to a resource in [`World`](crate::World).
///
/// Analogous to Bevy's `Res<T>`.
///
/// Appears in handler function signatures to declare a read dependency.
/// Derefs to the inner value transparently.
///
/// For exclusive write access, use [`ResMut<T>`]. For optional read
/// access (no panic if unregistered), use [`Option<Res<T>>`].
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
pub struct Res<'w, T: Resource> {
    value: &'w T,
}

impl<'w, T: Resource> Res<'w, T> {
    pub(crate) fn new(value: &'w T) -> Self {
        Self { value }
    }
}

impl<T: std::fmt::Debug + Resource> std::fmt::Debug for Res<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<T: Resource> Deref for Res<'_, T> {
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
/// Derefs to the inner value transparently.
///
/// For shared read access, use [`Res<T>`]. For optional write access
/// (no panic if unregistered), use [`Option<ResMut<T>>`].
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
pub struct ResMut<'w, T: Resource> {
    value: &'w mut T,
}

impl<'w, T: Resource> ResMut<'w, T> {
    pub(crate) fn new(value: &'w mut T) -> Self {
        Self { value }
    }
}

impl<T: std::fmt::Debug + Resource> std::fmt::Debug for ResMut<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<T: Resource> Deref for ResMut<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: Resource> DerefMut for ResMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
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
///     println!("event {} at sequence {}", event, seq.get());
/// }
/// ```
pub struct Seq(pub(crate) Sequence);

impl Seq {
    /// Returns the current sequence value.
    #[inline(always)]
    pub const fn get(self) -> i64 {
        self.0.get()
    }
}

impl Deref for Seq {
    type Target = Sequence;

    #[inline(always)]
    fn deref(&self) -> &Sequence {
        &self.0
    }
}

/// Mutable access to the world's current sequence number.
///
/// Allows handlers to advance the sequence — useful for stamping
/// outbound messages with monotonic sequence numbers.
///
/// # Example
///
/// ```ignore
/// use nexus_rt::{SeqMut, Handler, IntoHandler};
///
/// fn send_message(mut seq: SeqMut<'_>, event: u64) {
///     let msg_seq = seq.advance();
///     // stamp msg_seq on outbound message
/// }
/// ```
pub struct SeqMut<'w>(pub(crate) &'w Cell<Sequence>);

impl SeqMut<'_> {
    /// Returns the current sequence value.
    #[inline(always)]
    pub fn get(&self) -> Sequence {
        self.0.get()
    }

    /// Advance the sequence by 1 and return the new value.
    #[inline(always)]
    pub fn advance(&mut self) -> Sequence {
        let next = Sequence(self.0.get().0.wrapping_add(1));
        self.0.set(next);
        next
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Resource;

    struct Val(u64);
    impl Resource for Val {}

    #[test]
    fn res_deref() {
        let val = Val(42);
        let res = Res::new(&val);
        assert_eq!(res.0, 42);
    }

    #[test]
    fn res_mut_deref() {
        let mut val = Val(0);
        let mut res = ResMut::new(&mut val);
        res.0 = 99;
        assert_eq!(val.0, 99);
    }

    #[test]
    fn res_mut_deref_mut_no_stamp() {
        // ResMut::deref_mut is now a plain pass-through — no stamping.
        let mut val = Val(0);
        let mut res = ResMut::new(&mut val);
        *res = Val(123);
        assert_eq!(val.0, 123);
    }

    #[test]
    fn seq_get() {
        let seq = Seq(Sequence(42));
        assert_eq!(seq.get(), 42);
    }

    #[test]
    fn seq_mut_advance() {
        let cell = Cell::new(Sequence(0));
        let mut seq = SeqMut(&cell);
        let next = seq.advance();
        assert_eq!(next, Sequence(1));
        assert_eq!(cell.get(), Sequence(1));
    }
}
