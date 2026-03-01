//! Type-erased singleton resource storage.
//!
//! [`World`] is a unified store where each resource type gets a dense index
//! ([`ResourceId`]) for O(1) dispatch-time access. Registration happens through
//! [`WorldBuilder`], which freezes into an immutable [`World`] container via
//! [`build()`](WorldBuilder::build).
//!
//! The type [`Registry`] maps types to dense indices. It is shared between
//! [`WorldBuilder`] and [`World`], and is passed to [`SystemParam::init`] and
//! [`IntoSystem::into_system`] so that systems can resolve their parameter
//! state during driver setup — before or after `build()`.
//!
//! # Lifecycle
//!
//! ```text
//! let mut builder = WorldBuilder::new();
//! builder.register::<PriceCache>(value);
//! builder.register::<TimerDriver>(value);
//!
//! // Drivers can resolve systems against builder.registry()
//! // before World is built.
//!
//! let world = builder.build();  // → World (frozen)
//! ```
//!
//! After `build()`, the container is frozen — no inserts, no removes. All
//! [`ResourceId`] values are valid for the lifetime of the [`World`] container.

use std::any::{TypeId, type_name};
use std::cell::Cell;
use std::marker::PhantomData;

use rustc_hash::FxHashMap;

// =============================================================================
// Core types
// =============================================================================

/// Dense index identifying a resource type within a [`World`] container.
///
/// Assigned sequentially at registration (0, 1, 2, ...). Used as a direct
/// index into internal storage at dispatch time — no hashing, no searching.
///
/// Only obtained from [`Registry::id`], [`World::id`], or their `try_` variants.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ResourceId(usize);

/// Monotonic event sequence number for change detection.
///
/// Each event processed by a driver is assigned a unique sequence number
/// via [`World::next_sequence`]. Resources record the sequence at which
/// they were last written. A resource is considered "changed" if its
/// `changed_at` equals the world's `current_sequence`. Wrapping is
/// harmless — only equality is checked.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Sequence(pub(crate) u64);

/// Type-erased drop function. Monomorphized at registration time so we
/// can reconstruct and drop the original `Box<T>` from a `*mut u8`.
pub(crate) type DropFn = unsafe fn(*mut u8);

/// Reconstruct and drop a `Box<T>` from a raw pointer.
///
/// # Safety
///
/// `ptr` must have been produced by `Box::into_raw(Box::new(value))`
/// where `value: T`. Must only be called once per pointer.
pub(crate) unsafe fn drop_resource<T>(ptr: *mut u8) {
    // SAFETY: ptr was produced by Box::into_raw(Box::new(value))
    // where value: T. Called exactly once in Storage::drop.
    unsafe {
        let _ = Box::from_raw(ptr as *mut T);
    }
}

// =============================================================================
// Registry — type-to-index mapping
// =============================================================================

/// Type-to-index mapping shared between [`WorldBuilder`] and [`World`].
///
/// Contains only the type registry — no storage, no pointers. Passed to
/// [`IntoSystem::into_system`](crate::IntoSystem::into_system) and
/// [`SystemParam::init`](crate::SystemParam::init) so systems can resolve
/// [`ResourceId`]s during driver setup.
///
/// Obtained via [`WorldBuilder::registry()`] or [`World::registry()`].
pub struct Registry {
    indices: FxHashMap<TypeId, ResourceId>,
    /// Scratch bitset reused across [`check_access`](Self::check_access) calls.
    /// Allocated once on the first call with >64 resources, then reused.
    scratch: Vec<u64>,
}

impl Registry {
    pub(crate) fn new() -> Self {
        Self {
            indices: FxHashMap::default(),
            scratch: Vec::new(),
        }
    }

    /// Resolve the [`ResourceId`] for a type. Cold path — uses HashMap lookup.
    ///
    /// # Panics
    ///
    /// Panics if the resource type was not registered.
    pub fn id<T: 'static>(&self) -> ResourceId {
        *self
            .indices
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("resource `{}` not registered", type_name::<T>()))
    }

    /// Try to resolve the [`ResourceId`] for a type. Returns `None` if the
    /// type was not registered.
    pub fn try_id<T: 'static>(&self) -> Option<ResourceId> {
        self.indices.get(&TypeId::of::<T>()).copied()
    }

    /// Returns `true` if a resource of type `T` has been registered.
    pub fn contains<T: 'static>(&self) -> bool {
        self.indices.contains_key(&TypeId::of::<T>())
    }

    /// Returns the number of registered resources.
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Returns `true` if no resources have been registered.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Validate that a set of parameter accesses don't conflict.
    ///
    /// Two accesses conflict when they target the same ResourceId.
    /// Called at construction time by `into_system`, `into_callback`,
    /// and `into_stage`.
    ///
    /// Fast path (≤128 resources): single `u128` on the stack, zero heap.
    /// Slow path (>128 resources): reusable `Vec<u64>` owned by Registry —
    /// allocated once on first use, then cleared and reused.
    ///
    /// # Panics
    ///
    /// Panics if any resource is accessed by more than one parameter.
    #[cold]
    pub fn check_access(&mut self, accesses: &[(Option<ResourceId>, &str)]) {
        let n = self.len();
        if n == 0 {
            return;
        }

        if n <= 128 {
            // Fast path: single u128 on the stack.
            let mut seen = 0u128;
            for &(id, name) in accesses {
                let Some(id) = id else { continue };
                let bit = 1u128 << id.0;
                assert!(
                    seen & bit == 0,
                    "conflicting access: resource borrowed by `{}` is already \
                     borrowed by another parameter in the same system",
                    name,
                );
                seen |= bit;
            }
        } else {
            // Slow path: reusable heap buffer.
            let words = n.div_ceil(64);
            self.scratch.resize(words, 0);
            self.scratch.fill(0);
            for &(id, name) in accesses {
                let Some(id) = id else { continue };
                let word = id.0 / 64;
                let bit = 1u64 << (id.0 % 64);
                assert!(
                    self.scratch[word] & bit == 0,
                    "conflicting access: resource borrowed by `{}` is already \
                     borrowed by another parameter in the same system",
                    name,
                );
                self.scratch[word] |= bit;
            }
        }
    }
}

// =============================================================================
// Storage — shared backing between builder and frozen container
// =============================================================================

/// Interleaved pointer + change tick for a single resource.
/// 16 bytes — 4 slots per cache line.
#[repr(C)]
pub(crate) struct ResourceSlot {
    pub(crate) ptr: *mut u8,
    pub(crate) changed_at: Cell<Sequence>,
}

/// Internal storage for type-erased resource pointers and their destructors.
///
/// Owns the heap allocations and is responsible for cleanup. Shared between
/// [`WorldBuilder`] and [`World`] via move — avoids duplicating Drop logic.
pub(crate) struct Storage {
    /// Dense array of interleaved pointer + change tick pairs.
    /// Each pointer was produced by `Box::into_raw`.
    pub(crate) slots: Vec<ResourceSlot>,
    /// Parallel array of drop functions. `drop_fns[i]` is the monomorphized
    /// destructor for the concrete type behind `slots[i].ptr`.
    pub(crate) drop_fns: Vec<DropFn>,
}

impl Storage {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            drop_fns: Vec::new(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

// SAFETY: All values stored in Storage were registered via `register<T: Send + 'static>`,
// so every concrete type behind the raw pointers is Send. Storage exclusively owns
// these heap allocations — they are not aliased or shared. Transferring ownership
// to another thread is safe. Cell<Sequence> is !Sync but we're transferring
// ownership, not sharing.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for Storage {}

impl Drop for Storage {
    fn drop(&mut self) {
        for (slot, drop_fn) in self.slots.iter().zip(&self.drop_fns) {
            // Skip null pointers — defensive only. with_mut uses an RAII
            // guard to restore pointers on unwind, so null slots should
            // not occur in practice.
            if slot.ptr.is_null() {
                continue;
            }
            // SAFETY: each (slot.ptr, drop_fn) pair was created together in
            // WorldBuilder::register(). drop_fn is the monomorphized
            // destructor for the concrete type behind ptr. Called exactly
            // once here.
            unsafe {
                drop_fn(slot.ptr);
            }
        }
    }
}

// =============================================================================
// WorldBuilder
// =============================================================================

/// Builder for registering resources before freezing into a [`World`] container.
///
/// Each resource type can only be registered once. Registration assigns a
/// dense [`ResourceId`] index (0, 1, 2, ...).
///
/// The [`registry()`](Self::registry) method exposes the type-to-index mapping
/// so that drivers can resolve systems against the builder before `build()`.
///
/// # Examples
///
/// ```
/// use nexus_rt::WorldBuilder;
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(42);
/// builder.register::<bool>(true);
/// let world = builder.build();
///
/// let id = world.id::<u64>();
/// unsafe {
///     assert_eq!(*world.get::<u64>(id), 42);
/// }
/// ```
pub struct WorldBuilder {
    registry: Registry,
    storage: Storage,
}

impl WorldBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            registry: Registry::new(),
            storage: Storage::new(),
        }
    }

    /// Register a resource.
    ///
    /// The value is heap-allocated via `Box` and ownership is transferred
    /// to the container. The pointer is stable for the lifetime of the
    /// resulting [`World`].
    ///
    /// # Panics
    ///
    /// Panics if a resource of the same type is already registered.
    #[cold]
    pub fn register<T: Send + 'static>(&mut self, value: T) -> &mut Self {
        let type_id = TypeId::of::<T>();
        assert!(
            !self.registry.indices.contains_key(&type_id),
            "resource `{}` already registered",
            type_name::<T>(),
        );

        let ptr = Box::into_raw(Box::new(value)) as *mut u8;
        let id = ResourceId(self.storage.slots.len());
        self.registry.indices.insert(type_id, id);
        self.storage.slots.push(ResourceSlot {
            ptr,
            changed_at: Cell::new(Sequence(0)),
        });
        self.storage.drop_fns.push(drop_resource::<T>);
        self
    }

    /// Register a resource using its [`Default`] value.
    ///
    /// Equivalent to `self.register::<T>(T::default())`.
    #[cold]
    pub fn register_default<T: Default + Send + 'static>(&mut self) -> &mut Self {
        self.register(T::default())
    }

    /// Returns a shared reference to the type registry.
    ///
    /// Use this for read-only queries. For construction-time calls
    /// like [`into_system`](crate::IntoSystem::into_system), use
    /// [`registry_mut`](Self::registry_mut) instead.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Returns a mutable reference to the type registry.
    ///
    /// Needed at construction time for [`IntoSystem::into_system`],
    /// [`IntoCallback::into_callback`], and [`IntoStage::into_stage`]
    /// which call [`Registry::check_access`].
    pub fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }

    /// Returns the number of registered resources.
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns `true` if no resources have been registered.
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Returns `true` if a resource of type `T` has been registered.
    pub fn contains<T: 'static>(&self) -> bool {
        self.registry.contains::<T>()
    }

    /// Install a plugin. The plugin is consumed and registers its
    /// resources into this builder.
    pub fn install_plugin(&mut self, plugin: impl crate::plugin::Plugin) -> &mut Self {
        plugin.build(self);
        self
    }

    /// Install a driver. The driver is consumed, registers its resources
    /// into this builder, and returns a concrete handle for dispatch-time
    /// polling.
    pub fn install_driver<D: crate::driver::Driver>(&mut self, driver: D) -> D::Handle {
        driver.install(self)
    }

    /// Freeze the builder into an immutable [`World`] container.
    ///
    /// After this call, no more resources can be registered. All
    /// [`ResourceId`] values remain valid for the lifetime of the
    /// returned [`World`].
    pub fn build(self) -> World {
        World {
            registry: self.registry,
            storage: self.storage,
            current_sequence: Sequence(0),
            _not_sync: PhantomData,
        }
    }
}

impl Default for WorldBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// World — frozen container
// =============================================================================

/// Frozen singleton resource storage.
///
/// Created by [`WorldBuilder::build()`]. Resources are indexed by dense
/// [`ResourceId`] for O(1) dispatch-time access (~3 cycles per fetch).
///
/// # Safe API
///
/// - [`resource`](Self::resource) / [`resource_mut`](Self::resource_mut) —
///   cold-path access via HashMap lookup.
/// - [`with_mut`](Self::with_mut) — yanks one resource out of storage,
///   passes `(&mut T, &mut World)` to a closure. Systems dispatch
///   through `&mut World` safely.
///
/// # Unsafe API (framework internals)
///
/// The low-level `get` / `get_mut` methods are `unsafe` — used by
/// [`SystemParam::fetch`](crate::SystemParam) for ~3-cycle dispatch.
/// The caller must ensure no mutable aliasing.
pub struct World {
    /// Type-to-index mapping. Same registry used during build.
    registry: Registry,
    /// Type-erased pointer storage. Drop handled by `Storage`.
    storage: Storage,
    /// Current epoch tick. Advanced by the driver at the
    /// end of each dispatch pass.
    current_sequence: Sequence,
    /// World must not be shared across threads — it holds interior-mutable
    /// `Cell<Sequence>` values accessed through `&self`. `!Sync` enforced by
    /// `PhantomData<Cell<()>>`.
    _not_sync: PhantomData<Cell<()>>,
}

impl World {
    /// Convenience constructor — returns a new [`WorldBuilder`].
    pub fn builder() -> WorldBuilder {
        WorldBuilder::new()
    }

    /// Returns a shared reference to the type registry.
    ///
    /// Use this for read-only queries (e.g. [`id`](Registry::id),
    /// [`contains`](Registry::contains)). For construction-time calls
    /// like [`into_system`](crate::IntoSystem::into_system), use
    /// [`registry_mut`](Self::registry_mut) instead.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Returns a mutable reference to the type registry.
    ///
    /// Needed at construction time for [`IntoSystem::into_system`],
    /// [`IntoCallback::into_callback`], and [`IntoStage::into_stage`]
    /// which call [`Registry::check_access`].
    pub fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }

    /// Resolve the [`ResourceId`] for a type. Cold path — uses HashMap lookup.
    ///
    /// # Panics
    ///
    /// Panics if the resource type was not registered.
    pub fn id<T: 'static>(&self) -> ResourceId {
        self.registry.id::<T>()
    }

    /// Try to resolve the [`ResourceId`] for a type. Returns `None` if the
    /// type was not registered.
    pub fn try_id<T: 'static>(&self) -> Option<ResourceId> {
        self.registry.try_id::<T>()
    }

    /// Returns the number of registered resources.
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns `true` if no resources are stored.
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Returns `true` if a resource of type `T` is stored.
    pub fn contains<T: 'static>(&self) -> bool {
        self.registry.contains::<T>()
    }

    // =========================================================================
    // Safe resource access (cold path — HashMap lookup per call)
    // =========================================================================

    /// Safe shared access to a resource. Cold path — resolves via HashMap.
    ///
    /// Takes `&self` — multiple shared references can coexist. The borrow
    /// checker prevents mixing with [`resource_mut`](Self::resource_mut)
    /// or [`with_mut`](Self::with_mut) (both take `&mut self`).
    ///
    /// # Panics
    ///
    /// Panics if the resource type was not registered, or if the resource
    /// is currently borrowed by [`with_mut`](Self::with_mut).
    pub fn resource<T: 'static>(&self) -> &T {
        let id = self.registry.id::<T>();
        assert!(
            !self.storage.slots[id.0].ptr.is_null(),
            "resource `{}` is currently borrowed by with_mut",
            type_name::<T>(),
        );
        // SAFETY: id resolved from our own registry. &self prevents mutable
        // aliases — resource_mut/with_mut take &mut self. Null check above.
        unsafe { self.get(id) }
    }

    /// Safe exclusive access to a resource. Cold path — resolves via HashMap.
    ///
    /// # Panics
    ///
    /// Panics if the resource type was not registered, or if the resource
    /// is currently borrowed by [`with_mut`](Self::with_mut).
    pub fn resource_mut<T: 'static>(&mut self) -> &mut T {
        let id = self.registry.id::<T>();
        assert!(
            !self.storage.slots[id.0].ptr.is_null(),
            "resource `{}` is currently borrowed by with_mut",
            type_name::<T>(),
        );
        // Cold path — stamp unconditionally. If you request &mut, you're writing.
        self.storage.slots[id.0]
            .changed_at
            .set(self.current_sequence);
        // SAFETY: id resolved from our own registry. &mut self ensures
        // exclusive access — no other references can exist. Null check above.
        unsafe { self.get_mut(id) }
    }

    /// Borrow one resource mutably while passing `&mut World` for dispatch.
    ///
    /// The resource is temporarily removed from storage (pointer nulled)
    /// so that the closure receives `(&mut T, &mut World)` without aliasing.
    /// Safe resource accessors on `&mut World` will panic if called on the
    /// borrowed type — all other resources are accessible normally.
    ///
    /// The pointer is restored after the closure returns. If the closure
    /// panics, the pointer is restored via an RAII guard before unwinding
    /// continues — the resource remains accessible after `catch_unwind`.
    ///
    /// # Panics
    ///
    /// Panics if the resource type was not registered.
    pub fn with_mut<T: 'static, R>(&mut self, f: impl FnOnce(&mut T, &mut Self) -> R) -> R {
        let id = self.registry.id::<T>();
        assert!(
            !self.storage.slots[id.0].ptr.is_null(),
            "resource `{}` is currently borrowed by with_mut",
            type_name::<T>(),
        );
        // Cold path — stamp unconditionally. If you request &mut, you're writing.
        self.storage.slots[id.0]
            .changed_at
            .set(self.current_sequence);

        // Yank the pointer out — the closure cannot reach T through &mut World.
        let slot_ptr: *mut *mut u8 = &mut self.storage.slots[id.0].ptr;
        // SAFETY: slot_ptr is valid — derived from a bounds-checked index above.
        let ptr = std::mem::replace(unsafe { &mut *slot_ptr }, std::ptr::null_mut());

        // RAII guard restores the pointer on both normal return and panic unwind.
        struct RestoreGuard {
            slot_ptr: *mut *mut u8,
            original: *mut u8,
        }
        impl Drop for RestoreGuard {
            fn drop(&mut self) {
                // SAFETY: slot_ptr points into Storage::slots which outlives
                // this guard. original is the pointer yanked in with_mut.
                unsafe { *self.slot_ptr = self.original; }
            }
        }
        let _guard = RestoreGuard {
            slot_ptr,
            original: ptr,
        };

        // SAFETY: ptr was produced by Box::into_raw in register(), type T
        // matches. Removed from storage so no aliasing through &mut Self.
        // Null case excluded by assert above.
        let resource = unsafe { &mut *(ptr as *mut T) };
        f(resource, self)
    }

    // =========================================================================
    // Sequence / change detection
    // =========================================================================

    /// Returns the current event sequence number.
    pub fn current_sequence(&self) -> Sequence {
        self.current_sequence
    }

    /// Advance to the next event sequence number and return it.
    ///
    /// Drivers call this before dispatching each event. The returned
    /// sequence number identifies the event being processed. Resources
    /// mutated during dispatch will record this sequence in `changed_at`.
    pub fn next_sequence(&mut self) -> Sequence {
        self.current_sequence = Sequence(self.current_sequence.0.wrapping_add(1));
        self.current_sequence
    }

    // =========================================================================
    // Unsafe resource access (hot path — pre-resolved ResourceId)
    // =========================================================================

    /// Fetch a shared reference to a resource by pre-validated index.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`WorldBuilder::register`] for
    ///   the same builder that produced this container.
    /// - `T` must be the same type that was registered at this `id`.
    /// - The caller must ensure no mutable reference to this resource exists.
    #[inline(always)]
    pub unsafe fn get<T: 'static>(&self, id: ResourceId) -> &T {
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.slots.len().
        // T matches the registered type. No mutable alias exists.
        unsafe { &*(self.get_ptr(id) as *const T) }
    }

    /// Fetch a mutable reference to a resource by pre-validated index.
    ///
    /// Takes `&self` — the container structure is frozen, but individual
    /// resources have interior mutability via raw pointers. Sound because
    /// callers (single-threaded sequential dispatch) uphold no-aliasing.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`WorldBuilder::register`] for
    ///   the same builder that produced this container.
    /// - `T` must be the same type that was registered at this `id`.
    /// - The caller must ensure no other reference (shared or mutable) to this
    ///   resource exists.
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_mut<T: 'static>(&self, id: ResourceId) -> &mut T {
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.slots.len().
        // T matches the registered type. No aliases exist.
        unsafe { &mut *(self.get_ptr(id) as *mut T) }
    }

    /// Fetch a raw pointer to a resource by pre-validated index.
    ///
    /// Intended for macro-generated dispatch code that needs direct pointer
    /// access.
    ///
    /// # Safety
    ///
    /// - `id` must have been returned by [`WorldBuilder::register`] for
    ///   the same builder that produced this container.
    #[inline(always)]
    pub unsafe fn get_ptr(&self, id: ResourceId) -> *mut u8 {
        debug_assert!(
            id.0 < self.storage.slots.len(),
            "ResourceId({}) out of bounds (len {})",
            id.0,
            self.storage.slots.len(),
        );
        // SAFETY: caller guarantees id was returned by register() on the
        // builder that produced this container, so id.0 < self.storage.slots.len().
        let ptr = unsafe { self.storage.slots.get_unchecked(id.0).ptr };
        debug_assert!(
            !ptr.is_null(),
            "ResourceId({}) is null — resource is currently borrowed by with_mut",
            id.0,
        );
        ptr
    }

    // =========================================================================
    // Change-detection internals (framework use only)
    // =========================================================================

    /// Read the tick at which a resource was last changed.
    ///
    /// # Safety
    ///
    /// `id` must have been returned by [`WorldBuilder::register`] for
    /// the same builder that produced this container.
    #[inline(always)]
    pub(crate) unsafe fn changed_at(&self, id: ResourceId) -> Sequence {
        unsafe { self.storage.slots.get_unchecked(id.0).changed_at.get() }
    }

    /// Get a reference to the `Cell` tracking a resource's change tick.
    ///
    /// # Safety
    ///
    /// `id` must have been returned by [`WorldBuilder::register`] for
    /// the same builder that produced this container.
    #[inline(always)]
    pub(crate) unsafe fn changed_at_cell(&self, id: ResourceId) -> &Cell<Sequence> {
        unsafe { &self.storage.slots.get_unchecked(id.0).changed_at }
    }

    /// Stamp a resource as changed at the current tick.
    ///
    /// # Safety
    ///
    /// `id` must have been returned by [`WorldBuilder::register`] for
    /// the same builder that produced this container.
    #[inline(always)]
    #[allow(dead_code)] // Available for driver implementations.
    pub(crate) unsafe fn stamp_changed(&self, id: ResourceId) {
        unsafe {
            self.storage
                .slots
                .get_unchecked(id.0)
                .changed_at
                .set(self.current_sequence);
        }
    }
}

// SAFETY: All resources are `T: Send` (enforced by `register`). World owns all
// heap-allocated data exclusively — the raw pointers are not aliased or shared.
// Transferring ownership to another thread is safe; the new thread becomes the
// sole accessor.
unsafe impl Send for World {}

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
        let mut builder = WorldBuilder::new();
        builder
            .register::<Price>(Price { value: 100.0 })
            .register::<Venue>(Venue { name: "test" });
        let world = builder.build();
        assert_eq!(world.len(), 2);
    }

    #[test]
    fn resource_ids_are_sequential() {
        let mut builder = WorldBuilder::new();
        builder
            .register::<Price>(Price { value: 0.0 })
            .register::<Venue>(Venue { name: "" })
            .register::<Config>(Config { max_orders: 0 });
        let world = builder.build();
        assert_eq!(world.id::<Price>(), ResourceId(0));
        assert_eq!(world.id::<Venue>(), ResourceId(1));
        assert_eq!(world.id::<Config>(), ResourceId(2));
    }

    #[test]
    fn get_returns_registered_value() {
        let mut builder = WorldBuilder::new();
        builder.register::<Price>(Price { value: 42.5 });
        let world = builder.build();

        let id = world.id::<Price>();
        // SAFETY: id resolved from this container, type matches, no aliasing.
        let price = unsafe { world.get::<Price>(id) };
        assert_eq!(price.value, 42.5);
    }

    #[test]
    fn get_mut_modifies_value() {
        let mut builder = WorldBuilder::new();
        builder.register::<Price>(Price { value: 1.0 });
        let world = builder.build();

        let id = world.id::<Price>();
        // SAFETY: id resolved from this container, type matches, no aliasing.
        unsafe {
            world.get_mut::<Price>(id).value = 99.0;
            assert_eq!(world.get::<Price>(id).value, 99.0);
        }
    }

    #[test]
    fn try_id_returns_none_for_unregistered() {
        let world = WorldBuilder::new().build();
        assert!(world.try_id::<Price>().is_none());
    }

    #[test]
    fn try_id_returns_some_for_registered() {
        let mut builder = WorldBuilder::new();
        builder.register::<Price>(Price { value: 0.0 });
        let world = builder.build();

        assert!(world.try_id::<Price>().is_some());
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn panics_on_duplicate_registration() {
        let mut builder = WorldBuilder::new();
        builder.register::<Price>(Price { value: 1.0 });
        builder.register::<Price>(Price { value: 2.0 });
    }

    #[test]
    #[should_panic(expected = "not registered")]
    fn panics_on_unregistered_id() {
        let world = WorldBuilder::new().build();
        world.id::<Price>();
    }

    #[test]
    fn empty_builder_builds_empty_world() {
        let world = WorldBuilder::new().build();
        assert_eq!(world.len(), 0);
        assert!(world.is_empty());
    }

    #[test]
    fn drop_runs_destructors() {
        let arc = Arc::new(42u32);
        let weak: Weak<u32> = Arc::downgrade(&arc);

        {
            let mut builder = WorldBuilder::new();
            builder.register::<Arc<u32>>(arc);
            let _world = builder.build();
            // Arc still alive — held by World
            assert!(weak.upgrade().is_some());
        }
        // World dropped — Arc should be deallocated
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn builder_drop_cleans_up_without_build() {
        let arc = Arc::new(99u32);
        let weak: Weak<u32> = Arc::downgrade(&arc);

        {
            let mut builder = WorldBuilder::new();
            builder.register::<Arc<u32>>(arc);
        }
        // Builder dropped without build() — Storage::drop cleans up
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn multiple_types_independent() {
        let mut builder = WorldBuilder::new();
        builder
            .register::<Price>(Price { value: 10.0 })
            .register::<Venue>(Venue { name: "CB" })
            .register::<Config>(Config { max_orders: 500 });
        let world = builder.build();

        unsafe {
            let price_id = world.id::<Price>();
            let venue_id = world.id::<Venue>();
            let config_id = world.id::<Config>();
            assert_eq!(world.get::<Price>(price_id).value, 10.0);
            assert_eq!(world.get::<Venue>(venue_id).name, "CB");
            assert_eq!(world.get::<Config>(config_id).max_orders, 500);
        }
    }

    #[test]
    fn contains_reflects_registration() {
        let mut builder = WorldBuilder::new();
        assert!(!builder.contains::<Price>());

        builder.register::<Price>(Price { value: 0.0 });
        assert!(builder.contains::<Price>());
        assert!(!builder.contains::<Venue>());

        let world = builder.build();
        assert!(world.contains::<Price>());
        assert!(!world.contains::<Venue>());
    }

    #[test]
    fn get_ptr_returns_valid_pointer() {
        let mut builder = WorldBuilder::new();
        builder.register::<Price>(Price { value: 77.7 });
        let world = builder.build();

        let id = world.id::<Price>();
        unsafe {
            let ptr = world.get_ptr(id);
            let price = &*(ptr as *const Price);
            assert_eq!(price.value, 77.7);
        }
    }

    #[test]
    fn send_to_another_thread() {
        let mut builder = WorldBuilder::new();
        builder.register::<Price>(Price { value: 55.5 });
        let world = builder.build();

        let handle = std::thread::spawn(move || {
            let id = world.id::<Price>();
            // SAFETY: sole owner on this thread, no aliasing.
            unsafe { world.get::<Price>(id).value }
        });
        assert_eq!(handle.join().unwrap(), 55.5);
    }

    #[test]
    fn registry_accessible_from_builder() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);

        let registry = builder.registry();
        assert!(registry.contains::<u64>());
        assert!(!registry.contains::<bool>());

        let id = registry.id::<u64>();
        assert_eq!(id, ResourceId(0));
    }

    #[test]
    fn registry_accessible_from_world() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let world = builder.build();

        let registry = world.registry();
        assert!(registry.contains::<u64>());

        // Registry from world and world.id() agree.
        assert_eq!(registry.id::<u64>(), world.id::<u64>());
    }

    // -- Safe accessor tests --------------------------------------------------

    #[test]
    fn resource_reads_value() {
        let mut builder = WorldBuilder::new();
        builder.register::<Price>(Price { value: 42.5 });
        let world = builder.build();

        assert_eq!(world.resource::<Price>().value, 42.5);
    }

    #[test]
    fn resource_mut_modifies_value() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        *world.resource_mut::<u64>() = 99;
        assert_eq!(*world.resource::<u64>(), 99);
    }

    #[test]
    fn with_mut_provides_mutable_access_and_world() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(10).register::<bool>(false);
        let mut world = builder.build();

        world.with_mut::<u64, _>(|counter, world| {
            // Safe access to other resources through &mut World.
            let flag = world.resource::<bool>();
            if !flag {
                *counter += 5;
            }
        });

        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn with_mut_returns_value() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let mut world = builder.build();

        let val = world.with_mut::<u64, _>(|counter, _world| *counter);
        assert_eq!(val, 42);
    }

    #[test]
    #[should_panic(expected = "currently borrowed by with_mut")]
    fn with_mut_panics_on_aliased_access() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let mut world = builder.build();

        world.with_mut::<u64, _>(|_counter, world| {
            // Attempting to access the yanked resource through World panics.
            let _ = world.resource::<u64>();
        });
    }

    #[test]
    #[should_panic(expected = "currently borrowed by with_mut")]
    fn with_mut_panics_on_reentrant_borrow() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let mut world = builder.build();

        world.with_mut::<u64, _>(|_counter, world| {
            // Nested with_mut on the same type must panic, not UB.
            world.with_mut::<u64, _>(|_inner, _| {});
        });
    }

    #[test]
    fn with_mut_restores_pointer_after_panic() {
        use std::panic;

        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let mut world = builder.build();

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            world.with_mut::<u64, _>(|_counter, _world| {
                panic!("intentional");
            });
        }));
        assert!(result.is_err());

        // Resource must still be accessible — guard restored the pointer.
        assert_eq!(*world.resource::<u64>(), 42);
    }

    #[test]
    fn register_default_works() {
        use crate::event::Events;

        let mut builder = WorldBuilder::new();
        builder.register_default::<Events<u32>>();
        let world = builder.build();

        let events = world.resource::<Events<u32>>();
        assert!(events.is_empty());
    }

    // -- Sequence / change detection tests ----------------------------------------

    #[test]
    fn tick_default_is_zero() {
        assert_eq!(Sequence::default(), Sequence(0));
    }

    #[test]
    fn next_sequence_increments() {
        let mut world = WorldBuilder::new().build();
        assert_eq!(world.current_sequence(), Sequence(0));
        world.next_sequence();
        assert_eq!(world.current_sequence(), Sequence(1));
        world.next_sequence();
        assert_eq!(world.current_sequence(), Sequence(2));
    }

    #[test]
    fn resource_registered_at_current_sequence() {
        // Resources registered at build time get changed_at=Sequence(0).
        // World starts at current_sequence=Sequence(0). So they match — "changed."
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let world = builder.build();

        let id = world.id::<u64>();
        unsafe {
            assert_eq!(world.changed_at(id), Sequence(0));
            assert_eq!(world.current_sequence(), Sequence(0));
            // changed_at == current_sequence → "changed"
            assert_eq!(world.changed_at(id), world.current_sequence());
        }
    }

    #[test]
    fn resource_mut_stamps_changed_at() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        world.next_sequence(); // tick=1
        let id = world.id::<u64>();

        // changed_at is still 0, current_sequence is 1 → not changed
        unsafe {
            assert_eq!(world.changed_at(id), Sequence(0));
        }

        // resource_mut stamps changed_at to current_sequence
        *world.resource_mut::<u64>() = 99;
        unsafe {
            assert_eq!(world.changed_at(id), Sequence(1));
        }
    }

    #[test]
    fn with_mut_stamps_changed_at() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0).register::<bool>(false);
        let mut world = builder.build();

        world.next_sequence(); // tick=1
        let id = world.id::<u64>();

        unsafe {
            assert_eq!(world.changed_at(id), Sequence(0));
        }

        world.with_mut::<u64, _>(|val, _world| {
            *val = 42;
        });

        unsafe {
            assert_eq!(world.changed_at(id), Sequence(1));
        }
    }
}
