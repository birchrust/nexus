//! Build-time resolution and dispatch-time fetch for system parameters.
//!
//! The [`Fetch`] trait provides a two-phase pattern:
//!
//! 1. **Build time** — [`init`](Fetch::init) resolves opaque state (e.g., a
//!    [`ComponentId`](crate::ComponentId)) from a store. This panics if the
//!    required type isn't registered, giving an early build-time error.
//!
//! 2. **Dispatch time** — [`fetch`](Fetch::fetch) uses the cached state to
//!    produce a reference in ~3 cycles. No hashing, no searching.
//!
//! [`Comp`] and [`Ctx`] are the user-facing parameter wrappers that appear
//! in system function signatures. They deref to the inner value transparently.
//!
//! ```
//! # use nexus_rt::{Components, ComponentsBuilder, Fetch, Comp};
//! let components = ComponentsBuilder::new()
//!     .register::<u64>(42)
//!     .build();
//!
//! // Build time: resolve state
//! let state = <&u64 as Fetch<Components>>::init(&components);
//!
//! // Dispatch time: fetch reference
//! let val = unsafe { <&u64 as Fetch<Components>>::fetch(&components, &state) };
//! assert_eq!(*val, 42);
//! ```

use std::ops::{Deref, DerefMut};

use crate::components::{ComponentId, Components};
use crate::drivers::{DriverId, Drivers};

// =============================================================================
// Fetch trait
// =============================================================================

/// Resolves state at build time, fetches references at dispatch time.
///
/// Generic over `Source` — the store being fetched from ([`Components`] or
/// [`Drivers`]). Single-type only; arity (N parameters per system) is handled
/// at the system dispatch level.
///
/// # Safety
///
/// Implementors must ensure [`fetch`](Self::fetch) returns a valid reference
/// when given state produced by [`init`](Self::init) on the same source.
pub trait Fetch<Source> {
    /// Opaque state cached at build time (e.g., [`ComponentId`], [`DriverId`]).
    type State;

    /// Borrowed output at dispatch time.
    type Item<'a>
    where
        Source: 'a;

    /// Resolve state from the store. Called once at build time.
    ///
    /// # Panics
    ///
    /// Panics if the required type is not registered in `source`.
    fn init(source: &Source) -> Self::State;

    /// Fetch a reference using cached state.
    ///
    /// # Safety
    ///
    /// - `state` must have been produced by [`init`](Self::init) on the same
    ///   `source` instance.
    /// - Caller ensures no aliasing violations (at most one `&mut` per value).
    unsafe fn fetch<'a>(source: &'a Source, state: &Self::State) -> Self::Item<'a>;
}

// =============================================================================
// Reference impls via macro
// =============================================================================

macro_rules! impl_fetch_ref {
    ($source:ty, $id:ty) => {
        impl<T: 'static> Fetch<$source> for &T {
            type State = $id;
            type Item<'a> = &'a T;

            fn init(source: &$source) -> $id {
                source.id::<T>()
            }

            #[inline(always)]
            unsafe fn fetch<'a>(source: &'a $source, state: &$id) -> &'a T {
                // SAFETY: state was produced by init() on the same source.
                // Caller ensures no mutable alias exists.
                unsafe { source.get::<T>(*state) }
            }
        }

        impl<T: 'static> Fetch<$source> for &mut T {
            type State = $id;
            type Item<'a> = &'a mut T;

            fn init(source: &$source) -> $id {
                source.id::<T>()
            }

            #[inline(always)]
            unsafe fn fetch<'a>(source: &'a $source, state: &$id) -> &'a mut T {
                // SAFETY: state was produced by init() on the same source.
                // Caller ensures no aliases exist.
                unsafe { source.get_mut::<T>(*state) }
            }
        }
    };
}

impl_fetch_ref!(Components, ComponentId);
impl_fetch_ref!(Drivers, DriverId);

// =============================================================================
// Comp — component parameter wrapper
// =============================================================================

/// Component parameter — fetches from [`Components`].
///
/// Appears in system function signatures to declare a dependency on component
/// state. Derefs to the inner value transparently.
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
///
/// # Examples
///
/// ```
/// # use nexus_rt::{Components, ComponentsBuilder, Comp};
/// // In a system signature (future dispatch layer creates these):
/// // fn update_prices(prices: Comp<&mut PriceCache>) { ... }
///
/// // For now, manual construction (pub(crate)):
/// // let comp = Comp::new(unsafe { <&u64 as Fetch<Components>>::fetch(&c, &s) });
/// ```
pub struct Comp<'a, T: Fetch<Components>> {
    item: T::Item<'a>,
}

impl<'a, T: Fetch<Components>> Comp<'a, T> {
    /// Create a new `Comp` wrapping a fetched item.
    ///
    /// Only callable within the crate — the dispatch layer is responsible
    /// for constructing these with valid references.
    #[allow(dead_code)]
    pub(crate) fn new(item: T::Item<'a>) -> Self {
        Self { item }
    }
}

impl<T: 'static> Deref for Comp<'_, &T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.item
    }
}

impl<T: 'static> Deref for Comp<'_, &mut T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.item
    }
}

impl<T: 'static> DerefMut for Comp<'_, &mut T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        self.item
    }
}

// =============================================================================
// Ctx — driver context parameter wrapper
// =============================================================================

/// Driver context parameter — fetches from [`Drivers`].
///
/// Appears in system function signatures to declare a dependency on driver
/// state. Derefs to the inner value transparently.
///
/// Construction is `pub(crate)` — only the dispatch layer creates these.
///
/// # Examples
///
/// ```
/// # use nexus_rt::{Drivers, DriversBuilder, Ctx};
/// // In a system signature (future dispatch layer creates these):
/// // fn poll_io(io: Ctx<&mut IoRegistry>) { ... }
///
/// // For now, manual construction (pub(crate)):
/// // let ctx = Ctx::new(unsafe { <&Timer as Fetch<Drivers>>::fetch(&d, &s) });
/// ```
pub struct Ctx<'a, T: Fetch<Drivers>> {
    item: T::Item<'a>,
}

impl<'a, T: Fetch<Drivers>> Ctx<'a, T> {
    /// Create a new `Ctx` wrapping a fetched item.
    ///
    /// Only callable within the crate — the dispatch layer is responsible
    /// for constructing these with valid references.
    #[allow(dead_code)]
    pub(crate) fn new(item: T::Item<'a>) -> Self {
        Self { item }
    }
}

impl<T: 'static> Deref for Ctx<'_, &T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.item
    }
}

impl<T: 'static> Deref for Ctx<'_, &mut T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.item
    }
}

impl<T: 'static> DerefMut for Ctx<'_, &mut T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        self.item
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentsBuilder, DriversBuilder};

    struct PriceCache {
        value: f64,
    }

    struct TimerDriver {
        hz: u32,
    }

    struct IoRegistry {
        fd_count: usize,
    }

    // -- Fetch from Components ------------------------------------------------

    #[test]
    fn fetch_shared_ref() {
        let components = ComponentsBuilder::new()
            .register::<PriceCache>(PriceCache { value: 42.5 })
            .build();

        let state = <&PriceCache as Fetch<Components>>::init(&components);
        // SAFETY: state from init on same source, no aliasing.
        let price = unsafe { <&PriceCache as Fetch<Components>>::fetch(&components, &state) };
        assert_eq!(price.value, 42.5);
    }

    #[test]
    fn fetch_mut_ref() {
        let components = ComponentsBuilder::new()
            .register::<PriceCache>(PriceCache { value: 1.0 })
            .build();

        let state = <&mut PriceCache as Fetch<Components>>::init(&components);
        // SAFETY: state from init on same source, no aliasing.
        unsafe {
            let price = <&mut PriceCache as Fetch<Components>>::fetch(&components, &state);
            price.value = 99.0;
        }
        unsafe {
            let price = <&PriceCache as Fetch<Components>>::fetch(&components, &state);
            assert_eq!(price.value, 99.0);
        }
    }

    // -- Fetch from Drivers ---------------------------------------------------

    #[test]
    fn fetch_from_drivers() {
        let drivers = DriversBuilder::new()
            .register::<TimerDriver>(TimerDriver { hz: 1000 })
            .build();

        let state = <&TimerDriver as Fetch<Drivers>>::init(&drivers);
        // SAFETY: state from init on same source, no aliasing.
        let timer = unsafe { <&TimerDriver as Fetch<Drivers>>::fetch(&drivers, &state) };
        assert_eq!(timer.hz, 1000);
    }

    #[test]
    fn fetch_mut_from_drivers() {
        let drivers = DriversBuilder::new()
            .register::<IoRegistry>(IoRegistry { fd_count: 0 })
            .build();

        let state = <&mut IoRegistry as Fetch<Drivers>>::init(&drivers);
        // SAFETY: state from init on same source, no aliasing.
        unsafe {
            let io = <&mut IoRegistry as Fetch<Drivers>>::fetch(&drivers, &state);
            io.fd_count = 42;
        }
        unsafe {
            let io = <&IoRegistry as Fetch<Drivers>>::fetch(&drivers, &state);
            assert_eq!(io.fd_count, 42);
        }
    }

    // -- Comp deref -----------------------------------------------------------

    #[test]
    fn comp_deref() {
        let components = ComponentsBuilder::new()
            .register::<PriceCache>(PriceCache { value: 7.7 })
            .build();

        let state = <&PriceCache as Fetch<Components>>::init(&components);
        let item = unsafe { <&PriceCache as Fetch<Components>>::fetch(&components, &state) };
        let comp: Comp<&PriceCache> = Comp::new(item);
        assert_eq!(comp.value, 7.7);
    }

    #[test]
    fn comp_deref_mut() {
        let components = ComponentsBuilder::new()
            .register::<PriceCache>(PriceCache { value: 1.0 })
            .build();

        let state = <&mut PriceCache as Fetch<Components>>::init(&components);
        let item = unsafe { <&mut PriceCache as Fetch<Components>>::fetch(&components, &state) };
        let mut comp: Comp<&mut PriceCache> = Comp::new(item);
        comp.value = 88.0;
        assert_eq!(comp.value, 88.0);
    }

    // -- Ctx deref ------------------------------------------------------------

    #[test]
    fn ctx_deref() {
        let drivers = DriversBuilder::new()
            .register::<TimerDriver>(TimerDriver { hz: 500 })
            .build();

        let state = <&TimerDriver as Fetch<Drivers>>::init(&drivers);
        let item = unsafe { <&TimerDriver as Fetch<Drivers>>::fetch(&drivers, &state) };
        let ctx: Ctx<&TimerDriver> = Ctx::new(item);
        assert_eq!(ctx.hz, 500);
    }

    #[test]
    fn ctx_deref_mut() {
        let drivers = DriversBuilder::new()
            .register::<IoRegistry>(IoRegistry { fd_count: 10 })
            .build();

        let state = <&mut IoRegistry as Fetch<Drivers>>::init(&drivers);
        let item = unsafe { <&mut IoRegistry as Fetch<Drivers>>::fetch(&drivers, &state) };
        let mut ctx: Ctx<&mut IoRegistry> = Ctx::new(item);
        ctx.fd_count = 99;
        assert_eq!(ctx.fd_count, 99);
    }
}
