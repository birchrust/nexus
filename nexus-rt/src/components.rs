//! Type-erased singleton component storage.
//!
//! [`Components`] is an in-memory data store where each component type gets a
//! dense index ([`ComponentId`]) for O(1) dispatch-time access. Registration
//! happens through [`ComponentsBuilder`], which freezes into an immutable
//! [`Components`] container via [`build()`](ComponentsBuilder::build).
//!
//! # Lifecycle
//!
//! ```text
//! ComponentsBuilder::new()
//!     .register::<PriceCache>(value)
//!     .register::<VenueState>(value)
//!     .build()   // → Components (frozen)
//! ```
//!
//! After `build()`, the container is frozen — no inserts, no removes. All
//! [`ComponentId`] values are valid for the lifetime of the [`Components`]
//! container.

use std::any::{TypeId, type_name};
use std::collections::HashMap;

// =============================================================================
// Core types
// =============================================================================

/// Dense index identifying a component type within a [`Components`] container.
///
/// Assigned sequentially at registration (0, 1, 2, ...). Used as a direct
/// index into internal storage at dispatch time — no hashing, no searching.
///
/// Only obtained from [`Components::id`] or [`Components::try_id`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ComponentId(usize);

/// Type-erased drop function. Monomorphized at registration time so we
/// can reconstruct and drop the original `Box<T>` from a `*mut u8`.
type DropFn = unsafe fn(*mut u8);

/// Reconstruct and drop a `Box<T>` from a raw pointer.
///
/// # Safety
///
/// `ptr` must have been produced by `Box::into_raw(Box::new(value))`
/// where `value: T`. Must only be called once per pointer.
unsafe fn drop_component<T>(ptr: *mut u8) {
    // SAFETY: ptr was produced by Box::into_raw(Box::new(value))
    // where value: T. Called exactly once in Storage::drop.
    unsafe {
        let _ = Box::from_raw(ptr as *mut T);
    }
}

// =============================================================================
// Storage — shared backing between builder and frozen container
// =============================================================================

/// Internal storage for type-erased component pointers and their destructors.
///
/// Owns the heap allocations and is responsible for cleanup. Shared between
/// [`ComponentsBuilder`] and [`Components`] via move — avoids duplicating
/// Drop logic.
struct Storage {
    /// Dense array of type-erased pointers. Each was produced by `Box::into_raw`.
    ptrs: Vec<*mut u8>,
    /// Parallel array of drop functions. `drop_fns[i]` is the monomorphized
    /// destructor for the concrete type behind `ptrs[i]`.
    drop_fns: Vec<DropFn>,
}

impl Storage {
    fn new() -> Self {
        Self {
            ptrs: Vec::new(),
            drop_fns: Vec::new(),
        }
    }

    fn len(&self) -> usize {
        self.ptrs.len()
    }

    fn is_empty(&self) -> bool {
        self.ptrs.is_empty()
    }
}

// SAFETY: Storage exclusively owns the heap allocations behind its raw pointers.
// They are not aliased or shared. Transferring ownership to another thread is safe.
unsafe impl Send for Storage {}

impl Drop for Storage {
    fn drop(&mut self) {
        for (ptr, drop_fn) in self.ptrs.iter().zip(&self.drop_fns) {
            // SAFETY: each (ptr, drop_fn) pair was created together in
            // ComponentsBuilder::register(). drop_fn is the monomorphized
            // destructor for the concrete type behind ptr. Called exactly
            // once here.
            unsafe {
                drop_fn(*ptr);
            }
        }
    }
}

// =============================================================================
// ComponentsBuilder
// =============================================================================

/// Builder for registering components before freezing into a [`Components`]
/// container.
///
/// Each component type can only be registered once. Registration assigns a
/// dense [`ComponentId`] index (0, 1, 2, ...).
///
/// # Examples
///
/// ```
/// use nexus_rt::ComponentsBuilder;
///
/// let components = ComponentsBuilder::new()
///     .register::<u64>(42)
///     .register::<bool>(true)
///     .build();
///
/// let id = components.id::<u64>();
/// unsafe {
///     assert_eq!(*components.get::<u64>(id), 42);
/// }
/// ```
pub struct ComponentsBuilder {
    indices: HashMap<TypeId, ComponentId>,
    storage: Storage,
}

impl ComponentsBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            indices: HashMap::new(),
            storage: Storage::new(),
        }
    }

    /// Register a component.
    ///
    /// The value is heap-allocated via `Box` and ownership is transferred
    /// to the container. The pointer is stable for the lifetime of the
    /// resulting [`Components`]. Use [`Components::id`] after [`build()`](Self::build)
    /// to resolve the dense [`ComponentId`] for dispatch.
    ///
    /// # Panics
    ///
    /// Panics if a component of the same type is already registered.
    pub fn register<T: 'static>(mut self, value: T) -> Self {
        let type_id = TypeId::of::<T>();
        assert!(
            !self.indices.contains_key(&type_id),
            "component `{}` already registered",
            type_name::<T>(),
        );

        let ptr = Box::into_raw(Box::new(value)) as *mut u8;
        let id = ComponentId(self.storage.ptrs.len());
        self.indices.insert(type_id, id);
        self.storage.ptrs.push(ptr);
        self.storage.drop_fns.push(drop_component::<T>);
        self
    }

    /// Returns the number of registered components.
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns `true` if no components have been registered.
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Returns `true` if a component of type `T` has been registered.
    pub fn contains<T: 'static>(&self) -> bool {
        self.indices.contains_key(&TypeId::of::<T>())
    }

    /// Freeze the builder into an immutable [`Components`] container.
    ///
    /// After this call, no more components can be registered. All
    /// [`ComponentId`] values returned by [`register`](Self::register)
    /// remain valid for the lifetime of the returned [`Components`].
    pub fn build(self) -> Components {
        Components {
            indices: self.indices,
            storage: self.storage,
        }
    }
}

impl Default for ComponentsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Components — frozen container
// =============================================================================

/// Frozen singleton component storage.
///
/// Created by [`ComponentsBuilder::build()`]. Components are indexed by dense
/// [`ComponentId`] for O(1) dispatch-time access (~3 cycles per fetch).
///
/// # Safety
///
/// The `get` and `get_mut` methods are `unsafe` because the caller must
/// ensure no mutable aliasing occurs. This is sound in a single-threaded
/// sequential dispatch model where only one system accesses a component at
/// a time.
///
/// `get_mut` takes `&self` (not `&mut self`) because the container structure
/// is frozen — only the component values have interior mutability via raw
/// pointers. Same pattern as `UnsafeCell`.
pub struct Components {
    /// Build-time lookup: `TypeId` → dense index.
    indices: HashMap<TypeId, ComponentId>,
    /// Type-erased pointer storage. Drop handled by `Storage`.
    storage: Storage,
}

impl Components {
    /// Convenience constructor — returns a new [`ComponentsBuilder`].
    pub fn builder() -> ComponentsBuilder {
        ComponentsBuilder::new()
    }

    /// Resolve the [`ComponentId`] for a type. Cold path — uses HashMap lookup.
    ///
    /// # Panics
    ///
    /// Panics if the component type was not registered.
    pub fn id<T: 'static>(&self) -> ComponentId {
        *self
            .indices
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("component `{}` not registered", type_name::<T>()))
    }

    /// Try to resolve the [`ComponentId`] for a type. Returns `None` if the
    /// type was not registered.
    pub fn try_id<T: 'static>(&self) -> Option<ComponentId> {
        self.indices.get(&TypeId::of::<T>()).copied()
    }

    /// Returns the number of registered components.
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns `true` if no components are stored.
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Returns `true` if a component of type `T` is stored.
    pub fn contains<T: 'static>(&self) -> bool {
        self.indices.contains_key(&TypeId::of::<T>())
    }

    /// Fetch a shared reference to a component by pre-validated index.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`ComponentsBuilder::register`] for
    ///   the same builder that produced this container.
    /// - `T` must be the same type that was registered at this `id`.
    /// - The caller must ensure no mutable reference to this component exists.
    #[inline(always)]
    pub unsafe fn get<T: 'static>(&self, id: ComponentId) -> &T {
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.ptrs.len().
        // T matches the registered type. No mutable alias exists.
        unsafe { &*(self.get_ptr(id) as *const T) }
    }

    /// Fetch a mutable reference to a component by pre-validated index.
    ///
    /// Takes `&self` — the container structure is frozen, but individual
    /// components have interior mutability via raw pointers. Sound because
    /// callers (single-threaded sequential dispatch) uphold no-aliasing.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`ComponentsBuilder::register`] for
    ///   the same builder that produced this container.
    /// - `T` must be the same type that was registered at this `id`.
    /// - The caller must ensure no other reference (shared or mutable) to this
    ///   component exists.
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_mut<T: 'static>(&self, id: ComponentId) -> &mut T {
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.ptrs.len().
        // T matches the registered type. No aliases exist.
        unsafe { &mut *(self.get_ptr(id) as *mut T) }
    }

    /// Fetch a raw pointer to a component by pre-validated index.
    ///
    /// Intended for macro-generated dispatch code that needs direct pointer
    /// access.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`ComponentsBuilder::register`] for
    ///   the same builder that produced this container.
    #[inline(always)]
    pub unsafe fn get_ptr(&self, id: ComponentId) -> *mut u8 {
        debug_assert!(
            id.0 < self.storage.ptrs.len(),
            "ComponentId({}) out of bounds (len {})",
            id.0,
            self.storage.ptrs.len(),
        );
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.ptrs.len().
        unsafe { *self.storage.ptrs.get_unchecked(id.0) }
    }
}

// SAFETY: Components owns all heap-allocated data exclusively. The raw pointers
// in Storage are not shared — they were produced by Box::into_raw and are only
// accessed through &self/&mut self methods. Transferring ownership to another
// thread is safe; the new thread becomes the sole accessor.
unsafe impl Send for Components {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Weak};

    struct Price {
        value: f64,
    }

    struct Venue {
        name: &'static str,
    }

    struct Config {
        max_orders: usize,
    }

    #[test]
    fn register_and_build() {
        let components = ComponentsBuilder::new()
            .register::<Price>(Price { value: 100.0 })
            .register::<Venue>(Venue { name: "test" })
            .build();
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn component_ids_are_sequential() {
        let components = ComponentsBuilder::new()
            .register::<Price>(Price { value: 0.0 })
            .register::<Venue>(Venue { name: "" })
            .register::<Config>(Config { max_orders: 0 })
            .build();
        assert_eq!(components.id::<Price>(), ComponentId(0));
        assert_eq!(components.id::<Venue>(), ComponentId(1));
        assert_eq!(components.id::<Config>(), ComponentId(2));
    }

    #[test]
    fn get_returns_registered_value() {
        let components = ComponentsBuilder::new()
            .register::<Price>(Price { value: 42.5 })
            .build();

        let id = components.id::<Price>();
        // SAFETY: id resolved from this container, type matches, no aliasing.
        let price = unsafe { components.get::<Price>(id) };
        assert_eq!(price.value, 42.5);
    }

    #[test]
    fn get_mut_modifies_value() {
        let components = ComponentsBuilder::new()
            .register::<Price>(Price { value: 1.0 })
            .build();

        let id = components.id::<Price>();
        // SAFETY: id resolved from this container, type matches, no aliasing.
        unsafe {
            components.get_mut::<Price>(id).value = 99.0;
            assert_eq!(components.get::<Price>(id).value, 99.0);
        }
    }

    #[test]
    fn try_id_returns_none_for_unregistered() {
        let components = ComponentsBuilder::new().build();
        assert!(components.try_id::<Price>().is_none());
    }

    #[test]
    fn try_id_returns_some_for_registered() {
        let components = ComponentsBuilder::new()
            .register::<Price>(Price { value: 0.0 })
            .build();

        assert!(components.try_id::<Price>().is_some());
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn panics_on_duplicate_registration() {
        ComponentsBuilder::new()
            .register::<Price>(Price { value: 1.0 })
            .register::<Price>(Price { value: 2.0 });
    }

    #[test]
    #[should_panic(expected = "not registered")]
    fn panics_on_unregistered_id() {
        let components = ComponentsBuilder::new().build();
        components.id::<Price>();
    }

    #[test]
    fn empty_builder_builds_empty_components() {
        let components = ComponentsBuilder::new().build();
        assert_eq!(components.len(), 0);
        assert!(components.is_empty());
    }

    #[test]
    fn drop_runs_destructors() {
        let arc = Arc::new(42u32);
        let weak: Weak<u32> = Arc::downgrade(&arc);

        {
            let _components = ComponentsBuilder::new().register::<Arc<u32>>(arc).build();
            // Arc still alive — held by Components
            assert!(weak.upgrade().is_some());
        }
        // Components dropped — Arc should be deallocated
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn builder_drop_cleans_up_without_build() {
        let arc = Arc::new(99u32);
        let weak: Weak<u32> = Arc::downgrade(&arc);

        {
            // register() consumes and returns builder — binding keeps it alive
            let _builder = ComponentsBuilder::new().register::<Arc<u32>>(arc);
        }
        // Builder dropped without build() — Storage::drop cleans up
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn multiple_types_independent() {
        let components = ComponentsBuilder::new()
            .register::<Price>(Price { value: 10.0 })
            .register::<Venue>(Venue { name: "CB" })
            .register::<Config>(Config { max_orders: 500 })
            .build();

        unsafe {
            let price_id = components.id::<Price>();
            let venue_id = components.id::<Venue>();
            let config_id = components.id::<Config>();
            assert_eq!(components.get::<Price>(price_id).value, 10.0);
            assert_eq!(components.get::<Venue>(venue_id).name, "CB");
            assert_eq!(components.get::<Config>(config_id).max_orders, 500);
        }
    }

    #[test]
    fn contains_reflects_registration() {
        let builder = ComponentsBuilder::new();
        assert!(!builder.contains::<Price>());

        let builder = builder.register::<Price>(Price { value: 0.0 });
        assert!(builder.contains::<Price>());
        assert!(!builder.contains::<Venue>());

        let components = builder.build();
        assert!(components.contains::<Price>());
        assert!(!components.contains::<Venue>());
    }

    #[test]
    fn get_ptr_returns_valid_pointer() {
        let components = ComponentsBuilder::new()
            .register::<Price>(Price { value: 77.7 })
            .build();

        let id = components.id::<Price>();
        unsafe {
            let ptr = components.get_ptr(id);
            let price = &*(ptr as *const Price);
            assert_eq!(price.value, 77.7);
        }
    }

    #[test]
    fn send_to_another_thread() {
        let components = ComponentsBuilder::new()
            .register::<Price>(Price { value: 55.5 })
            .build();

        let handle = std::thread::spawn(move || {
            let id = components.id::<Price>();
            // SAFETY: sole owner on this thread, no aliasing.
            unsafe { components.get::<Price>(id).value }
        });
        assert_eq!(handle.join().unwrap(), 55.5);
    }
}
