//! Shared and mutable resource references for system parameters.

use std::ops::{Deref, DerefMut};

/// Shared reference to a resource in [`World`](crate::World).
///
/// Appears in system function signatures to declare a read dependency.
/// Derefs to the inner value transparently.
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
pub struct Res<'w, T: 'static> {
    value: &'w T,
}

impl<'w, T: 'static> Res<'w, T> {
    pub(crate) fn new(value: &'w T) -> Self {
        Self { value }
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
/// Derefs to the inner value transparently.
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
pub struct ResMut<'w, T: 'static> {
    value: &'w mut T,
}

impl<'w, T: 'static> ResMut<'w, T> {
    pub(crate) fn new(value: &'w mut T) -> Self {
        Self { value }
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
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn res_deref() {
        let val = 42u64;
        let res = Res::new(&val);
        assert_eq!(*res, 42);
    }

    #[test]
    fn res_mut_deref_mut() {
        let mut val = 1u64;
        let mut res = ResMut::new(&mut val);
        *res = 99;
        assert_eq!(*res, 99);
        drop(res);
        assert_eq!(val, 99);
    }
}
