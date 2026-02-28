//! Shared and mutable resource references for system parameters.

use std::cell::Cell;
use std::ops::{Deref, DerefMut};

use crate::world::Tick;

/// Shared reference to a resource in [`World`](crate::World).
///
/// Appears in system function signatures to declare a read dependency.
/// Derefs to the inner value transparently. Carries change-detection
/// metadata — call [`is_changed`](Self::is_changed) to check.
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
pub struct Res<'w, T: 'static> {
    value: &'w T,
    changed_at: Tick,
    current_tick: Tick,
}

impl<'w, T: 'static> Res<'w, T> {
    pub(crate) fn new(value: &'w T, changed_at: Tick, current_tick: Tick) -> Self {
        Self {
            value,
            changed_at,
            current_tick,
        }
    }

    /// Returns `true` if the resource was modified during the current tick.
    pub fn is_changed(&self) -> bool {
        self.changed_at == self.current_tick
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
/// `changed_at` tick on [`DerefMut`] — the act of writing is the
/// change signal.
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
pub struct ResMut<'w, T: 'static> {
    value: &'w mut T,
    changed_at: &'w Cell<Tick>,
    current_tick: Tick,
}

impl<'w, T: 'static> ResMut<'w, T> {
    pub(crate) fn new(value: &'w mut T, changed_at: &'w Cell<Tick>, current_tick: Tick) -> Self {
        Self {
            value,
            changed_at,
            current_tick,
        }
    }

    /// Returns `true` if the resource was modified during the current tick.
    pub fn is_changed(&self) -> bool {
        self.changed_at.get() == self.current_tick
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
        self.changed_at.set(self.current_tick);
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn res_deref() {
        let val = 42u64;
        let res = Res::new(&val, Tick::default(), Tick::default());
        assert_eq!(*res, 42);
    }

    #[test]
    fn res_is_changed() {
        let val = 42u64;
        let tick = Tick::default();
        let res = Res::new(&val, tick, tick);
        assert!(res.is_changed());
    }

    #[test]
    fn res_not_changed() {
        let val = 42u64;
        // changed_at=0, current_tick=1 → not changed
        let res = Res::new(&val, Tick::default(), Tick(1));
        assert!(!res.is_changed());
    }

    #[test]
    fn res_mut_deref_mut() {
        let mut val = 1u64;
        let changed_at = Cell::new(Tick::default());
        let mut res = ResMut::new(&mut val, &changed_at, Tick::default());
        *res = 99;
        assert_eq!(*res, 99);
        drop(res);
        assert_eq!(val, 99);
    }

    #[test]
    fn res_mut_deref_mut_stamps() {
        let mut val = 1u64;
        let changed_at = Cell::new(Tick(0));
        let current = Tick(5);
        let mut res = ResMut::new(&mut val, &changed_at, current);

        // Before DerefMut — changed_at is still 0
        assert_eq!(changed_at.get(), Tick(0));

        *res = 99;

        // After DerefMut — changed_at stamped to current_tick
        assert_eq!(changed_at.get(), Tick(5));
    }

    #[test]
    fn res_mut_deref_does_not_stamp() {
        let mut val = 42u64;
        let changed_at = Cell::new(Tick(0));
        let current = Tick(5);
        let res = ResMut::new(&mut val, &changed_at, current);

        // Deref (shared) — read only, should not stamp
        let _ = *res;
        assert_eq!(changed_at.get(), Tick(0));
    }

    #[test]
    fn res_mut_is_changed() {
        let mut val = 1u64;
        let changed_at = Cell::new(Tick(3));
        let res = ResMut::new(&mut val, &changed_at, Tick(3));
        assert!(res.is_changed());

        let res2 = ResMut::new(&mut val, &changed_at, Tick(4));
        assert!(!res2.is_changed());
    }
}
