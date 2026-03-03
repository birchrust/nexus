//! Shared and mutable resource references for system parameters.

use std::cell::Cell;
use std::ops::{Deref, DerefMut};

use crate::world::Sequence;

/// Shared reference to a resource in [`World`](crate::World).
///
/// Appears in system function signatures to declare a read dependency.
/// Derefs to the inner value transparently. Carries change-detection
/// metadata — call [`is_changed`](Self::is_changed) to check.
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
/// Appears in system function signatures to declare a write dependency.
/// Derefs to the inner value transparently. Stamps the resource's
/// `changed_at` sequence on [`DerefMut`] — the act of writing is the
/// change signal.
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
    fn res_mut_is_changed() {
        let mut val = 1u64;
        let changed_at = Cell::new(Sequence(3));
        let res = ResMut::new(&mut val, &changed_at, Sequence(3));
        assert!(res.is_changed());

        let res2 = ResMut::new(&mut val, &changed_at, Sequence(4));
        assert!(!res2.is_changed());
    }
}
