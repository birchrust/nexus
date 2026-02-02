//! Macro for creating TLS-based global allocators.
//!
//! The `create_allocator!` macro generates a module with thread-local slab
//! storage and a lightweight `Slot` handle (8 bytes).
//!
//! # Example
//!
//! ```ignore
//! use nexus_slab::create_allocator;
//!
//! // Create an allocator for the Order type
//! create_allocator!(order_alloc, Order);
//!
//! // Initialize at boot time
//! order_alloc::init().bounded(1024).build();
//!
//! // Use the allocator
//! let slot = order_alloc::insert(Order::new());
//! let key = slot.key();
//! ```

/// Creates a TLS-based allocator module for a specific type.
///
/// # Syntax
///
/// ```ignore
/// create_allocator!(module_name, Type);
/// ```
///
/// # Generated API
///
/// The macro generates a module with:
///
/// - `init() -> UnconfiguredBuilder` - Start configuring the allocator
/// - `insert(value: T) -> Slot` - Insert a value (panics if not initialized or full)
/// - `try_insert(value: T) -> Option<Slot>` - Insert a value (returns None if full)
/// - `contains_key(key: Key) -> bool` - Check if key is valid
/// - `unsafe fn get_unchecked(key: Key) -> &'static T` - Get reference by key
/// - `unsafe fn get_unchecked_mut(key: Key) -> &'static mut T` - Get mutable reference
/// - `shutdown() -> Result<(), SlotsRemaining>` - Shutdown the allocator
/// - `Slot` - RAII handle (8 bytes)
///
/// # Builder Pattern
///
/// ```ignore
/// // Bounded slab (fixed capacity)
/// my_alloc::init()
///     .bounded(1024)
///     .build();
///
/// // Unbounded slab (growable)
/// my_alloc::init()
///     .unbounded()
///     .chunk_capacity(4096)
///     .capacity(10_000)  // pre-allocate
///     .build();
/// ```
///
/// # Safety
///
/// **Do not store `Slot` in `thread_local!`**. Rust drops stack variables before
/// TLS, so slots on the stack will drop correctly. But if both Slot and the slab
/// are in TLS, drop order is unspecified and may cause UB.
///
/// # Thread Safety
///
/// Each thread has its own allocator instance. The allocator is `!Send` and `!Sync`.
#[macro_export]
macro_rules! create_allocator {
    ($name:ident, $T:ty) => {
        pub mod $name {
            use std::cell::{Cell, RefCell};

            use $crate::shared::{ClaimedSlot, SlotCell, VTable, VACANT_BIT};
            use $crate::Key;

            // =================================================================
            // TLS Storage
            // =================================================================

            /// Internal storage enum for bounded vs unbounded.
            enum SlabStorage {
                Bounded($crate::bounded::BoundedSlabInner<$T>),
                Unbounded(Box<$crate::unbounded::SlabInner<$T>>),
            }

            thread_local! {
                /// The slab storage. Only touched during init/shutdown.
                static SLAB: RefCell<Option<SlabStorage>> = const { RefCell::new(None) };

                /// Cached VTable pointer for fast hot-path access.
                static VTABLE: Cell<*const VTable<$T>> = const { Cell::new(std::ptr::null()) };
            }

            // =================================================================
            // Slot (8 bytes)
            // =================================================================

            /// RAII handle to an occupied slot.
            ///
            /// When dropped, the slot is returned to the freelist.
            /// Use [`leak()`](Self::leak) to keep the data alive.
            ///
            /// # Size
            ///
            /// 8 bytes (single pointer). The VTable is looked up from TLS.
            #[must_use = "dropping Slot deallocates the slot"]
            pub struct Slot {
                slot: *mut SlotCell<$T>,
            }

            impl Slot {
                #[inline]
                fn slot(&self) -> &SlotCell<$T> {
                    // SAFETY: Slot holds a valid slot pointer
                    unsafe { &*self.slot }
                }

                /// Returns the key for this slot.
                #[inline]
                pub fn key(&self) -> Key {
                    Key::new(self.slot().key_from_stamp())
                }

                /// Leaks the slot, keeping the data alive and returning its key.
                ///
                /// After calling `leak()`, the slot remains occupied but has no
                /// Slot owner. Access the data via key-based functions.
                #[inline]
                pub fn leak(self) -> Key {
                    let key = self.key();
                    std::mem::forget(self);
                    key
                }

                /// Returns a reference to the value.
                #[inline]
                pub fn get(&self) -> &$T {
                    // SAFETY: Slot owns the slot. SlotCell is repr(C): [stamp: 8][value: T]
                    unsafe { &*((self.slot as *const u8).add(8) as *const $T) }
                }

                /// Returns a mutable reference to the value.
                ///
                /// Requires `&mut Slot` for exclusive access.
                #[inline]
                pub fn get_mut(&mut self) -> &mut $T {
                    // SAFETY: Slot owns the slot, &mut ensures exclusivity.
                    unsafe { &mut *((self.slot as *mut u8).add(8) as *mut $T) }
                }

                /// Returns a raw pointer to the value.
                #[inline]
                pub fn as_ptr(&self) -> *const $T {
                    unsafe { (self.slot as *const u8).add(8) as *const $T }
                }

                /// Returns a mutable raw pointer to the value.
                #[inline]
                pub fn as_mut_ptr(&mut self) -> *mut $T {
                    unsafe { (self.slot as *mut u8).add(8) as *mut $T }
                }

                /// Returns `true` if the slot is still occupied.
                #[inline]
                pub fn is_valid(&self) -> bool {
                    let stamp = unsafe { *(self.slot as *const u64) };
                    stamp & VACANT_BIT == 0
                }

                /// Replaces the value, returning the old one.
                #[inline]
                pub fn replace(&mut self, value: $T) -> $T {
                    let value_ptr = unsafe { (self.slot as *mut u8).add(8) as *mut $T };
                    let old = unsafe { value_ptr.read() };
                    unsafe { value_ptr.write(value) };
                    old
                }

                /// Consumes the slot, returning the value and deallocating.
                #[inline]
                pub fn into_inner(self) -> $T {
                    let slot = self.slot();
                    let key = Key::new(slot.key_from_stamp());

                    // SAFETY: Slot owns the slot
                    let value = unsafe {
                        let value_ptr = (self.slot as *const u8).add(8) as *const $T;
                        std::ptr::read(value_ptr)
                    };

                    // Free the slot
                    VTABLE.with(|v| {
                        let vtable = v.get();
                        debug_assert!(!vtable.is_null(), "allocator not initialized - call init().build() first");
                        // SAFETY: VTable is valid, slot was occupied
                        unsafe {
                            let vtable = &*vtable;
                            (vtable.free_fn)(vtable.inner, key);
                        }
                    });

                    std::mem::forget(self);
                    value
                }
            }

            impl Drop for Slot {
                fn drop(&mut self) {
                    let slot = self.slot();
                    let key = Key::new(slot.key_from_stamp());

                    // Drop the value
                    unsafe {
                        let value_ptr = (self.slot as *mut u8).add(8) as *mut $T;
                        std::ptr::drop_in_place(value_ptr);
                    }

                    // Free the slot via VTable
                    VTABLE.with(|v| {
                        let vtable = v.get();
                        debug_assert!(!vtable.is_null(), "allocator not initialized - call init().build() first");
                        // SAFETY: VTable is valid
                        unsafe {
                            let vtable = &*vtable;
                            (vtable.free_fn)(vtable.inner, key);
                        }
                    });
                }
            }

            impl std::ops::Deref for Slot {
                type Target = $T;

                #[inline]
                fn deref(&self) -> &Self::Target {
                    self.get()
                }
            }

            impl std::ops::DerefMut for Slot {
                #[inline]
                fn deref_mut(&mut self) -> &mut Self::Target {
                    self.get_mut()
                }
            }

            impl std::fmt::Debug for Slot {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.debug_struct("Slot").field("key", &self.key()).finish()
                }
            }

            // =================================================================
            // Builder Pattern
            // =================================================================

            /// Builder in unconfigured state.
            pub struct UnconfiguredBuilder {
                _private: (),
            }

            impl UnconfiguredBuilder {
                /// Configure for a bounded (fixed capacity) slab.
                #[inline]
                pub fn bounded(self, capacity: usize) -> BoundedBuilder {
                    BoundedBuilder { capacity }
                }

                /// Configure for an unbounded (growable) slab.
                #[inline]
                pub fn unbounded(self) -> UnboundedBuilder {
                    UnboundedBuilder {
                        chunk_capacity: 4096,
                        initial_capacity: 0,
                    }
                }
            }

            /// Builder for bounded slab.
            pub struct BoundedBuilder {
                capacity: usize,
            }

            impl BoundedBuilder {
                /// Build and install the allocator.
                ///
                /// # Panics
                ///
                /// Panics if allocator is already initialized.
                pub fn build(self) {
                    SLAB.with(|s| {
                        let mut slab = s.borrow_mut();
                        assert!(slab.is_none(), "allocator already initialized");

                        // Create the bounded slab inner
                        let inner = $crate::bounded::BoundedSlabInner::<$T>::with_capacity(
                            self.capacity as u32,
                        );

                        // Create and leak the VTable
                        let mut vtable = $crate::bounded::BoundedSlabInner::<$T>::vtable();

                        // Store the slab
                        *slab = Some(SlabStorage::Bounded(inner));

                        // Get pointer to inner and set it in vtable
                        if let Some(SlabStorage::Bounded(ref inner)) = *slab {
                            let inner_ptr = inner as *const _ as *mut ();
                            // SAFETY: Setting inner pointer
                            unsafe { vtable.set_inner(inner_ptr) };
                        }

                        // Leak the vtable and store pointer
                        let vtable_ptr = Box::leak(Box::new(vtable));
                        VTABLE.with(|v| v.set(vtable_ptr));
                    });
                }
            }

            /// Builder for unbounded slab.
            pub struct UnboundedBuilder {
                chunk_capacity: usize,
                initial_capacity: usize,
            }

            impl UnboundedBuilder {
                /// Set chunk capacity (default: 4096, rounded to power of 2).
                #[inline]
                pub fn chunk_capacity(mut self, cap: usize) -> Self {
                    self.chunk_capacity = cap;
                    self
                }

                /// Pre-allocate space for this many items (default: 0).
                #[inline]
                pub fn capacity(mut self, cap: usize) -> Self {
                    self.initial_capacity = cap;
                    self
                }

                /// Build and install the allocator.
                ///
                /// # Panics
                ///
                /// Panics if allocator is already initialized.
                pub fn build(self) {
                    SLAB.with(|s| {
                        let mut slab = s.borrow_mut();
                        assert!(slab.is_none(), "allocator already initialized");

                        // Create the unbounded slab inner
                        let inner = Box::new($crate::unbounded::SlabInner::<$T>::with_chunk_capacity(
                            self.chunk_capacity,
                        ));

                        // Pre-allocate if requested
                        if self.initial_capacity > 0 {
                            while inner.capacity() < self.initial_capacity {
                                inner.grow();
                            }
                        }

                        // Store the slab FIRST (before getting pointer)
                        *slab = Some(SlabStorage::Unbounded(inner));

                        // Create VTable and get pointer from stored location
                        let mut vtable = $crate::unbounded::SlabInner::<$T>::vtable();

                        // Get pointer to inner from stored location
                        if let Some(SlabStorage::Unbounded(ref inner)) = *slab {
                            let inner_ptr = &**inner as *const _ as *mut ();
                            // SAFETY: Setting inner pointer to stable heap location
                            unsafe { vtable.set_inner(inner_ptr) };
                        }

                        // Leak the vtable and store pointer
                        let vtable_ptr = Box::leak(Box::new(vtable));
                        VTABLE.with(|v| v.set(vtable_ptr));
                    });
                }
            }

            // =================================================================
            // Public API
            // =================================================================

            /// Start configuring the allocator.
            ///
            /// # Example
            ///
            /// ```ignore
            /// my_alloc::init().bounded(1024).build();
            /// ```
            #[inline]
            pub fn init() -> UnconfiguredBuilder {
                UnconfiguredBuilder { _private: () }
            }

            /// Insert a value, returning an RAII Slot.
            ///
            /// # Panics
            ///
            /// Panics if the allocator is not initialized or is full (bounded only).
            #[inline]
            pub fn insert(value: $T) -> Slot {
                try_insert(value).expect("allocator full or not initialized")
            }

            /// Try to insert a value.
            ///
            /// Returns `None` if the allocator is full.
            ///
            /// # Panics (debug builds)
            ///
            /// Panics if the allocator is not initialized.
            #[inline]
            pub fn try_insert(value: $T) -> Option<Slot> {
                VTABLE.with(|v| {
                    let vtable = v.get();
                    debug_assert!(!vtable.is_null(), "allocator not initialized - call init().build() first");

                    // SAFETY: Caller must have called init().build() before any operations.
                    // Debug builds: panic above. Release builds: null deref crash.
                    // We don't add a runtime check here to avoid hot path overhead.
                    // Use is_initialized() if you need to check at runtime.
                    let vtable = unsafe { &*vtable };

                    // Try to claim a slot
                    let claimed = unsafe { (vtable.try_claim_fn)(vtable.inner) }?;

                    // Write the value
                    let slot = claimed.slot_ptr as *mut SlotCell<$T>;
                    unsafe {
                        let value_ptr = (slot as *mut u8).add(8) as *mut $T;
                        std::ptr::write(value_ptr, value);
                        // Mark as occupied
                        (*slot).set_key_occupied(claimed.key.index());
                    }

                    Some(Slot { slot })
                })
            }

            /// Check if a key refers to a valid, occupied slot.
            ///
            /// # Panics (debug builds)
            ///
            /// Panics if the allocator is not initialized.
            #[inline]
            pub fn contains_key(key: Key) -> bool {
                VTABLE.with(|v| {
                    let vtable = v.get();
                    debug_assert!(!vtable.is_null(), "allocator not initialized - call init().build() first");

                    // SAFETY: VTable is valid (debug_assert guards this in debug, null deref in release)
                    unsafe {
                        let vtable = &*vtable;
                        (vtable.contains_key_fn)(vtable.inner, key)
                    }
                })
            }

            /// Get a reference to a value by key.
            ///
            /// # Safety
            ///
            /// - Key must refer to an occupied slot
            /// - No mutable references may exist to this slot
            #[inline]
            pub unsafe fn get_unchecked(key: Key) -> &'static $T {
                VTABLE.with(|v| {
                    let vtable = v.get();
                    debug_assert!(!vtable.is_null(), "allocator not initialized - call init().build() first");
                    let vtable = unsafe { &*vtable };
                    let slot_ptr = unsafe { (vtable.slot_ptr_fn)(vtable.inner, key) };
                    // SlotCell is repr(C): [stamp: 8][value: T]
                    unsafe { &*((slot_ptr as *const u8).add(8) as *const $T) }
                })
            }

            /// Get a mutable reference to a value by key.
            ///
            /// # Safety
            ///
            /// - Key must refer to an occupied slot
            /// - No other references may exist to this slot
            #[inline]
            #[allow(clippy::mut_from_ref)]
            pub unsafe fn get_unchecked_mut(key: Key) -> &'static mut $T {
                VTABLE.with(|v| {
                    let vtable = v.get();
                    debug_assert!(!vtable.is_null(), "allocator not initialized - call init().build() first");
                    let vtable = unsafe { &*vtable };
                    let slot_ptr = unsafe { (vtable.slot_ptr_fn)(vtable.inner, key) };
                    // SlotCell is repr(C): [stamp: 8][value: T]
                    unsafe { &mut *((slot_ptr as *mut u8).add(8) as *mut $T) }
                })
            }

            /// Error returned when shutdown is called with slots still in use.
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct SlotsRemaining(pub usize);

            impl std::fmt::Display for SlotsRemaining {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{} slots still in use", self.0)
                }
            }

            impl std::error::Error for SlotsRemaining {}

            /// Shutdown the allocator.
            ///
            /// Returns an error if any slots are still in use.
            pub fn shutdown() -> Result<(), SlotsRemaining> {
                SLAB.with(|s| {
                    let mut slab = s.borrow_mut();
                    if slab.is_none() {
                        return Ok(());
                    }

                    // Check if any slots are in use
                    let len = match &*slab {
                        Some(SlabStorage::Bounded(inner)) => inner.len() as usize,
                        Some(SlabStorage::Unbounded(inner)) => inner.len(),
                        None => 0,
                    };

                    if len > 0 {
                        return Err(SlotsRemaining(len));
                    }

                    // Clear the slab
                    *slab = None;

                    // Clear the vtable pointer (but don't deallocate - it's leaked)
                    VTABLE.with(|v| v.set(std::ptr::null()));

                    Ok(())
                })
            }

            /// Returns the number of occupied slots.
            #[inline]
            pub fn len() -> usize {
                SLAB.with(|s| {
                    let slab = s.borrow();
                    match &*slab {
                        Some(SlabStorage::Bounded(inner)) => inner.len() as usize,
                        Some(SlabStorage::Unbounded(inner)) => inner.len(),
                        None => 0,
                    }
                })
            }

            /// Returns true if no slots are occupied.
            #[inline]
            pub fn is_empty() -> bool {
                len() == 0
            }

            /// Returns the current capacity.
            #[inline]
            pub fn capacity() -> usize {
                SLAB.with(|s| {
                    let slab = s.borrow();
                    match &*slab {
                        Some(SlabStorage::Bounded(inner)) => inner.capacity() as usize,
                        Some(SlabStorage::Unbounded(inner)) => inner.capacity(),
                        None => 0,
                    }
                })
            }

            /// Returns true if the allocator is initialized.
            #[inline]
            pub fn is_initialized() -> bool {
                VTABLE.with(|v| !v.get().is_null())
            }
        }
    };
}

// Tests are in tests/macro_tests.rs to avoid visibility issues with macro-generated modules

