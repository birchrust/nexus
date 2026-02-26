//! Type-erased singleton driver storage.
//!
//! [`Drivers`] is a store for driver objects (timer drivers, IO registries, etc.)
//! using the same type-erased dense-index pattern as [`Components`](crate::Components).
//! Systems belong to drivers — the dispatching driver determines which systems
//! run and when.
//!
//! # Lifecycle
//!
//! ```text
//! DriversBuilder::new()
//!     .register::<TimerDriver>(value)
//!     .register::<IoRegistry>(value)
//!     .build()   // → Drivers (frozen)
//! ```
//!
//! After `build()`, the container is frozen — no inserts, no removes. All
//! [`DriverId`] values are valid for the lifetime of the [`Drivers`] container.

use std::any::{TypeId, type_name};
use std::collections::HashMap;

use crate::components::{Storage, drop_component};

// =============================================================================
// Core types
// =============================================================================

/// Dense index identifying a driver type within a [`Drivers`] container.
///
/// Assigned sequentially at registration (0, 1, 2, ...). Used as a direct
/// index into internal storage at dispatch time — no hashing, no searching.
///
/// Only obtained from [`Drivers::id`] or [`Drivers::try_id`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DriverId(usize);

// =============================================================================
// DriversBuilder
// =============================================================================

/// Builder for registering drivers before freezing into a [`Drivers`] container.
///
/// Each driver type can only be registered once. Registration assigns a dense
/// [`DriverId`] index (0, 1, 2, ...).
///
/// # Examples
///
/// ```
/// use nexus_rt::DriversBuilder;
///
/// struct TimerDriver { hz: u32 }
///
/// let drivers = DriversBuilder::new()
///     .register::<TimerDriver>(TimerDriver { hz: 1000 })
///     .build();
///
/// let id = drivers.id::<TimerDriver>();
/// unsafe {
///     assert_eq!(drivers.get::<TimerDriver>(id).hz, 1000);
/// }
/// ```
pub struct DriversBuilder {
    indices: HashMap<TypeId, DriverId>,
    storage: Storage,
}

impl DriversBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            indices: HashMap::new(),
            storage: Storage::new(),
        }
    }

    /// Register a driver.
    ///
    /// The value is heap-allocated via `Box` and ownership is transferred
    /// to the container. The pointer is stable for the lifetime of the
    /// resulting [`Drivers`]. Use [`Drivers::id`] after [`build()`](Self::build)
    /// to resolve the dense [`DriverId`] for dispatch.
    ///
    /// # Panics
    ///
    /// Panics if a driver of the same type is already registered.
    pub fn register<T: 'static>(mut self, value: T) -> Self {
        let type_id = TypeId::of::<T>();
        assert!(
            !self.indices.contains_key(&type_id),
            "driver `{}` already registered",
            type_name::<T>(),
        );

        let ptr = Box::into_raw(Box::new(value)) as *mut u8;
        let id = DriverId(self.storage.ptrs.len());
        self.indices.insert(type_id, id);
        self.storage.ptrs.push(ptr);
        self.storage.drop_fns.push(drop_component::<T>);
        self
    }

    /// Returns the number of registered drivers.
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns `true` if no drivers have been registered.
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Returns `true` if a driver of type `T` has been registered.
    pub fn contains<T: 'static>(&self) -> bool {
        self.indices.contains_key(&TypeId::of::<T>())
    }

    /// Freeze the builder into an immutable [`Drivers`] container.
    ///
    /// After this call, no more drivers can be registered. All [`DriverId`]
    /// values remain valid for the lifetime of the returned [`Drivers`].
    pub fn build(self) -> Drivers {
        Drivers {
            indices: self.indices,
            storage: self.storage,
        }
    }
}

impl Default for DriversBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Drivers — frozen container
// =============================================================================

/// Frozen singleton driver storage.
///
/// Created by [`DriversBuilder::build()`]. Drivers are indexed by dense
/// [`DriverId`] for O(1) dispatch-time access (~3 cycles per fetch).
///
/// # Safety
///
/// The `get` and `get_mut` methods are `unsafe` because the caller must
/// ensure no mutable aliasing occurs. This is sound in a single-threaded
/// sequential dispatch model where only one system accesses a driver at
/// a time.
///
/// `get_mut` takes `&self` (not `&mut self`) because the container structure
/// is frozen — only the driver values have interior mutability via raw
/// pointers. Same pattern as `UnsafeCell`.
pub struct Drivers {
    /// Build-time lookup: `TypeId` → dense index.
    indices: HashMap<TypeId, DriverId>,
    /// Type-erased pointer storage. Drop handled by `Storage`.
    storage: Storage,
}

impl Drivers {
    /// Convenience constructor — returns a new [`DriversBuilder`].
    pub fn builder() -> DriversBuilder {
        DriversBuilder::new()
    }

    /// Resolve the [`DriverId`] for a type. Cold path — uses HashMap lookup.
    ///
    /// # Panics
    ///
    /// Panics if the driver type was not registered.
    pub fn id<T: 'static>(&self) -> DriverId {
        *self
            .indices
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("driver `{}` not registered", type_name::<T>()))
    }

    /// Try to resolve the [`DriverId`] for a type. Returns `None` if the
    /// type was not registered.
    pub fn try_id<T: 'static>(&self) -> Option<DriverId> {
        self.indices.get(&TypeId::of::<T>()).copied()
    }

    /// Returns the number of registered drivers.
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns `true` if no drivers are stored.
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Returns `true` if a driver of type `T` is stored.
    pub fn contains<T: 'static>(&self) -> bool {
        self.indices.contains_key(&TypeId::of::<T>())
    }

    /// Fetch a shared reference to a driver by pre-validated index.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`DriversBuilder::register`] for
    ///   the same builder that produced this container.
    /// - `T` must be the same type that was registered at this `id`.
    /// - The caller must ensure no mutable reference to this driver exists.
    #[inline(always)]
    pub unsafe fn get<T: 'static>(&self, id: DriverId) -> &T {
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.ptrs.len().
        // T matches the registered type. No mutable alias exists.
        unsafe { &*(self.get_ptr(id) as *const T) }
    }

    /// Fetch a mutable reference to a driver by pre-validated index.
    ///
    /// Takes `&self` — the container structure is frozen, but individual
    /// drivers have interior mutability via raw pointers. Sound because
    /// callers (single-threaded sequential dispatch) uphold no-aliasing.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`DriversBuilder::register`] for
    ///   the same builder that produced this container.
    /// - `T` must be the same type that was registered at this `id`.
    /// - The caller must ensure no other reference (shared or mutable) to this
    ///   driver exists.
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_mut<T: 'static>(&self, id: DriverId) -> &mut T {
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.ptrs.len().
        // T matches the registered type. No aliases exist.
        unsafe { &mut *(self.get_ptr(id) as *mut T) }
    }

    /// Fetch a raw pointer to a driver by pre-validated index.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`DriversBuilder::register`] for
    ///   the same builder that produced this container.
    #[inline(always)]
    pub unsafe fn get_ptr(&self, id: DriverId) -> *mut u8 {
        debug_assert!(
            id.0 < self.storage.ptrs.len(),
            "DriverId({}) out of bounds (len {})",
            id.0,
            self.storage.ptrs.len(),
        );
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.ptrs.len().
        unsafe { *self.storage.ptrs.get_unchecked(id.0) }
    }
}

// SAFETY: Drivers owns all heap-allocated data exclusively. The raw pointers
// in Storage are not shared — they were produced by Box::into_raw and are only
// accessed through &self methods. Transferring ownership to another thread is safe;
// the new thread becomes the sole accessor.
unsafe impl Send for Drivers {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Weak};

    struct TimerDriver {
        hz: u32,
    }

    struct IoRegistry {
        _name: &'static str,
    }

    struct NetDriver {
        _mtu: usize,
    }

    #[test]
    fn register_and_build() {
        let drivers = DriversBuilder::new()
            .register::<TimerDriver>(TimerDriver { hz: 1000 })
            .register::<IoRegistry>(IoRegistry { _name: "epoll" })
            .build();
        assert_eq!(drivers.len(), 2);
    }

    #[test]
    fn driver_ids_are_sequential() {
        let drivers = DriversBuilder::new()
            .register::<TimerDriver>(TimerDriver { hz: 0 })
            .register::<IoRegistry>(IoRegistry { _name: "" })
            .register::<NetDriver>(NetDriver { _mtu: 0 })
            .build();
        assert_eq!(drivers.id::<TimerDriver>(), DriverId(0));
        assert_eq!(drivers.id::<IoRegistry>(), DriverId(1));
        assert_eq!(drivers.id::<NetDriver>(), DriverId(2));
    }

    #[test]
    fn get_returns_registered_value() {
        let drivers = DriversBuilder::new()
            .register::<TimerDriver>(TimerDriver { hz: 500 })
            .build();

        let id = drivers.id::<TimerDriver>();
        // SAFETY: id resolved from this container, type matches, no aliasing.
        let timer = unsafe { drivers.get::<TimerDriver>(id) };
        assert_eq!(timer.hz, 500);
    }

    #[test]
    fn get_mut_modifies_value() {
        let drivers = DriversBuilder::new()
            .register::<TimerDriver>(TimerDriver { hz: 1 })
            .build();

        let id = drivers.id::<TimerDriver>();
        // SAFETY: id resolved from this container, type matches, no aliasing.
        unsafe {
            drivers.get_mut::<TimerDriver>(id).hz = 9999;
            assert_eq!(drivers.get::<TimerDriver>(id).hz, 9999);
        }
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn panics_on_duplicate_registration() {
        DriversBuilder::new()
            .register::<TimerDriver>(TimerDriver { hz: 1 })
            .register::<TimerDriver>(TimerDriver { hz: 2 });
    }

    #[test]
    #[should_panic(expected = "not registered")]
    fn panics_on_unregistered_id() {
        let drivers = DriversBuilder::new().build();
        drivers.id::<TimerDriver>();
    }

    #[test]
    fn empty_builder_builds_empty_drivers() {
        let drivers = DriversBuilder::new().build();
        assert_eq!(drivers.len(), 0);
        assert!(drivers.is_empty());
    }

    #[test]
    fn drop_runs_destructors() {
        let arc = Arc::new(42u32);
        let weak: Weak<u32> = Arc::downgrade(&arc);

        {
            let _drivers = DriversBuilder::new().register::<Arc<u32>>(arc).build();
            assert!(weak.upgrade().is_some());
        }
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn builder_drop_cleans_up_without_build() {
        let arc = Arc::new(99u32);
        let weak: Weak<u32> = Arc::downgrade(&arc);

        {
            let _builder = DriversBuilder::new().register::<Arc<u32>>(arc);
        }
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn send_to_another_thread() {
        let drivers = DriversBuilder::new()
            .register::<TimerDriver>(TimerDriver { hz: 750 })
            .build();

        let handle = std::thread::spawn(move || {
            let id = drivers.id::<TimerDriver>();
            // SAFETY: sole owner on this thread, no aliasing.
            unsafe { drivers.get::<TimerDriver>(id).hz }
        });
        assert_eq!(handle.join().unwrap(), 750);
    }
}
