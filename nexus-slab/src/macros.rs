//! Allocator macros and builder types.
//!
//! This module provides:
//! - [`bounded_allocator!`] - generates a bounded (fixed-capacity) allocator
//! - [`unbounded_allocator!`] - generates an unbounded (growable) allocator
//!
//! # Usage Pattern
//!
//! ```ignore
//! // In your types module, create an allocator submodule
//! pub mod alloc {
//!     nexus_slab::bounded_allocator!(super::Order);
//! }
//!
//! // Initialize at startup
//! alloc::Allocator::builder()
//!     .capacity(10_000)
//!     .build()?;
//!
//! // Use like Box
//! let slot = alloc::Slot::new(Order { ... })?;
//! println!("{}", slot.price);  // Deref
//! ```

use std::error::Error;
use std::fmt;

// =============================================================================
// Error Types
// =============================================================================

/// Error returned when attempting to initialize an already-initialized allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyInitialized;

impl fmt::Display for AlreadyInitialized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "allocator already initialized")
    }
}

impl Error for AlreadyInitialized {}

// =============================================================================
// Builder Types
// =============================================================================

/// Builder for bounded allocator configuration.
///
/// Created by `Allocator::builder()` in macro-generated code.
#[derive(Debug)]
pub struct BoundedBuilder {
    capacity: Option<usize>,
}

impl BoundedBuilder {
    /// Creates a new builder.
    #[inline]
    pub const fn new() -> Self {
        Self { capacity: None }
    }

    /// Sets the capacity (required).
    ///
    /// # Panics
    ///
    /// `build()` will panic if capacity is not set.
    #[inline]
    pub const fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    /// Returns the configured capacity.
    ///
    /// # Panics
    ///
    /// Panics if capacity was not set.
    #[inline]
    pub fn get_capacity(&self) -> usize {
        self.capacity.expect("capacity must be set before build()")
    }
}

impl Default for BoundedBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for unbounded allocator configuration.
///
/// Created by `Allocator::builder()` in macro-generated code.
#[derive(Debug)]
pub struct UnboundedBuilder {
    chunk_size: usize,
}

impl UnboundedBuilder {
    /// Default chunk size (4096 slots per chunk).
    pub const DEFAULT_CHUNK_SIZE: usize = 4096;

    /// Creates a new builder with default settings.
    #[inline]
    pub const fn new() -> Self {
        Self {
            chunk_size: Self::DEFAULT_CHUNK_SIZE,
        }
    }

    /// Sets the chunk size (optional, defaults to 4096).
    ///
    /// Chunks are allocated when the allocator grows. Larger chunks
    /// mean fewer allocations but more memory per growth step.
    #[inline]
    pub const fn chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Returns the configured chunk size.
    #[inline]
    pub const fn get_chunk_size(&self) -> usize {
        self.chunk_size
    }
}

impl Default for UnboundedBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Macros
// =============================================================================

/// Generates a bounded (fixed-capacity) slab allocator for a type.
///
/// This macro creates a module with:
/// - `Allocator` - unit struct with static methods for lifecycle management
/// - `Slot` - type alias for [`alloc::Slot<Allocator>`](crate::alloc::Slot)
///
/// # Example
///
/// ```ignore
/// // Define your type
/// pub struct Order {
///     pub id: u64,
///     pub price: f64,
/// }
///
/// // Create allocator module
/// pub mod alloc {
///     nexus_slab::bounded_allocator!(super::Order);
/// }
///
/// // Initialize at startup
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     alloc::Allocator::builder()
///         .capacity(10_000)
///         .build()?;
///
///     // Use
///     let slot = alloc::Slot::new(Order { id: 1, price: 100.0 });
///     println!("Order price: {}", slot.price);  // Deref
///
///     Ok(())
/// }
/// ```
#[macro_export]
macro_rules! bounded_allocator {
    ($ty:ty) => {
        use $crate::Key;
        use $crate::SlotCell;
        use $crate::bounded::SlabInner as BoundedSlabInner;
        use $crate::macros::AlreadyInitialized;

        thread_local! {
            static INNER: BoundedSlabInner<$ty> = const { BoundedSlabInner::new() };
        }

        /// Unit struct providing static methods for allocator lifecycle.
        pub struct Allocator;

        /// Builder for configuring this allocator.
        pub struct Builder {
            capacity: Option<usize>,
        }

        impl Builder {
            /// Sets the capacity (required).
            #[inline]
            pub fn capacity(mut self, capacity: usize) -> Self {
                self.capacity = Some(capacity);
                self
            }

            /// Builds and initializes the allocator.
            ///
            /// # Errors
            ///
            /// Returns `AlreadyInitialized` if the allocator was already initialized.
            ///
            /// # Panics
            ///
            /// Panics if capacity was not set.
            pub fn build(self) -> Result<(), AlreadyInitialized> {
                let capacity = self.capacity.expect("capacity must be set before build()");
                INNER.with(|inner| {
                    if inner.is_initialized() {
                        return Err(AlreadyInitialized);
                    }
                    inner.init(capacity as u32);
                    Ok(())
                })
            }
        }

        impl Allocator {
            /// Returns a builder for configuring the allocator.
            #[inline]
            pub fn builder() -> Builder {
                Builder { capacity: None }
            }

            /// Returns true if the allocator has been initialized.
            #[inline]
            pub fn is_initialized() -> bool {
                INNER.with(|inner| inner.is_initialized())
            }

            /// Returns the total capacity.
            #[inline]
            pub fn capacity() -> usize {
                INNER.with(|inner| inner.capacity() as usize)
            }
        }

        // SAFETY: try_alloc claims from the TLS freelist, writes value via union.
        // dealloc/dealloc_ptr return the slot to the TLS freelist via union field write.
        // slot_cell returns a valid pointer for any key within capacity.
        // All operations are single-threaded (TLS).
        unsafe impl $crate::Alloc for Allocator {
            type Item = $ty;

            #[inline]
            fn try_alloc(
                value: Self::Item,
            ) -> Result<*mut SlotCell<$ty>, $crate::alloc::Full<$ty>> {
                // Pop from freelist — value stays on caller's stack, not captured
                let slot = INNER.with(|inner| {
                    let slot_ptr = inner.free_head.get();
                    // SAFETY: slot_ptr came from the freelist. Slot is vacant,
                    // so next_free is the active union field.
                    let slot_nn = std::ptr::NonNull::new(slot_ptr)?;
                    let next_free = unsafe { (*slot_nn.as_ptr()).next_free };
                    inner.free_head.set(next_free);
                    Some(slot_nn)
                });

                match slot {
                    Some(slot_nn) => {
                        let slot_ptr = slot_nn.as_ptr();
                        // Write value outside closure — single memcpy from stack to slot
                        // SAFETY: Slot is claimed from freelist, we have exclusive access
                        unsafe {
                            (*slot_ptr).value =
                                std::mem::ManuallyDrop::new(std::mem::MaybeUninit::new(value));
                        }
                        Ok(slot_ptr)
                    }
                    None => Err($crate::alloc::Full(value)),
                }
            }

            unsafe fn dealloc(key: Key) {
                INNER.with(|inner| {
                    let slot_ptr = inner.slots_ptr().add(key.index() as usize);

                    // Write freelist link — overwrites value bytes (union semantics)
                    let free_head = inner.free_head.get();
                    (*slot_ptr).next_free = free_head;
                    inner.free_head.set(slot_ptr);
                });
            }

            #[inline]
            unsafe fn dealloc_ptr(slot_ptr: *mut SlotCell<$ty>) {
                INNER.with(|inner| {
                    // Write freelist link — overwrites value bytes (union semantics)
                    // SAFETY: Caller guarantees slot_ptr is valid and within this slab
                    let free_head = inner.free_head.get();
                    (*slot_ptr).next_free = free_head;
                    inner.free_head.set(slot_ptr);
                });
            }

            #[inline]
            unsafe fn slot_cell(key: Key) -> *mut SlotCell<$ty> {
                INNER.with(|inner| inner.slots_ptr().add(key.index() as usize))
            }

            fn slot_index(slot_ptr: *mut SlotCell<$ty>) -> Key {
                INNER.with(|inner| {
                    // SAFETY: slot_ptr came from this slab's allocation
                    let index = unsafe { inner.slot_to_index(slot_ptr) };
                    Key::new(index)
                })
            }
        }

        impl $crate::BoundedAlloc for Allocator {}

        // Borrow/BorrowMut must be implemented here (not generically) because
        // the blanket `impl<T> Borrow<T> for T` conflicts with generic impls.
        impl std::borrow::Borrow<$ty> for $crate::alloc::Slot<Allocator> {
            #[inline]
            fn borrow(&self) -> &$ty {
                self
            }
        }

        impl std::borrow::BorrowMut<$ty> for $crate::alloc::Slot<Allocator> {
            #[inline]
            fn borrow_mut(&mut self) -> &mut $ty {
                self
            }
        }

        /// RAII handle to a slab-allocated value.
        ///
        /// Type alias for [`alloc::Slot<Allocator>`](crate::alloc::Slot).
        pub type Slot = $crate::alloc::Slot<Allocator>;
    };
}

/// Generates an unbounded (growable) slab allocator for a type.
///
/// This macro creates a module with:
/// - `Allocator` - unit struct with static methods for lifecycle management
/// - `Slot` - type alias for [`alloc::Slot<Allocator>`](crate::alloc::Slot)
///
/// Unlike `bounded_allocator!`, `Slot::new()` always succeeds by growing
/// the allocator as needed.
///
/// # Example
///
/// ```ignore
/// pub mod alloc {
///     nexus_slab::unbounded_allocator!(super::Quote);
/// }
///
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     alloc::Allocator::builder()
///         .chunk_size(4096)
///         .build()?;
///
///     // Always succeeds - grows as needed
///     let slot = alloc::Slot::new(Quote { ... });
///
///     Ok(())
/// }
/// ```
#[macro_export]
macro_rules! unbounded_allocator {
    ($ty:ty) => {
        use $crate::Key;
        use $crate::SlotCell;
        use $crate::macros::AlreadyInitialized;
        use $crate::unbounded::SlabInner;

        /// Default chunk size (4096 slots per chunk).
        const DEFAULT_CHUNK_SIZE: usize = 4096;

        thread_local! {
            static INNER: SlabInner<$ty> = const { SlabInner::new() };
        }

        /// Unit struct providing static methods for allocator lifecycle.
        pub struct Allocator;

        /// Builder for configuring this allocator.
        pub struct Builder {
            chunk_size: usize,
        }

        impl Builder {
            /// Sets the chunk size (optional, defaults to 4096).
            ///
            /// Chunks are allocated when the allocator grows. Larger chunks
            /// mean fewer allocations but more memory per growth step.
            #[inline]
            pub fn chunk_size(mut self, size: usize) -> Self {
                self.chunk_size = size;
                self
            }

            /// Builds and initializes the allocator.
            ///
            /// # Errors
            ///
            /// Returns `AlreadyInitialized` if the allocator was already initialized.
            pub fn build(self) -> Result<(), AlreadyInitialized> {
                INNER.with(|inner| {
                    if inner.is_initialized() {
                        return Err(AlreadyInitialized);
                    }
                    inner.init(self.chunk_size as u32);
                    Ok(())
                })
            }
        }

        impl Allocator {
            /// Returns a builder for configuring the allocator.
            #[inline]
            pub fn builder() -> Builder {
                Builder {
                    chunk_size: DEFAULT_CHUNK_SIZE,
                }
            }

            /// Returns true if the allocator has been initialized.
            #[inline]
            pub fn is_initialized() -> bool {
                INNER.with(|inner| inner.is_initialized())
            }

            /// Returns the total capacity across all chunks.
            #[inline]
            pub fn capacity() -> usize {
                INNER.with(|inner| inner.capacity())
            }
        }

        // SAFETY: try_alloc grows the slab if needed, claims from chunk freelist,
        // writes value via union. dealloc/dealloc_ptr return slot to correct
        // chunk freelist via union field write and handle chunk availability tracking.
        // slot_cell decodes the key and returns a valid pointer.
        // All operations are single-threaded (TLS).
        unsafe impl $crate::Alloc for Allocator {
            type Item = $ty;

            #[inline]
            fn alloc(value: Self::Item) -> *mut SlotCell<$ty> {
                // Pop from freelist — value stays on caller's stack, not captured
                let slot_ptr = INNER.with(|inner| {
                    // Grow if needed (unbounded always succeeds)
                    if inner.head_with_space_is_none() {
                        inner.grow();
                    }

                    let (_chunk_idx, chunk) = inner.head_chunk();
                    let chunk_inner = chunk.inner_ref();

                    // Pop from chunk's freelist
                    let slot_ptr = chunk_inner.free_head.get();
                    debug_assert!(!slot_ptr.is_null(), "chunk on freelist has no free slots");

                    // SAFETY: slot_ptr came from the freelist. Slot is vacant,
                    // so next_free is the active union field.
                    let next_free = unsafe { (*slot_ptr).next_free };

                    // Update chunk's freelist head
                    chunk_inner.free_head.set(next_free);

                    // If chunk is now full, remove from available-chunk list
                    if next_free.is_null() {
                        inner.pop_head_chunk();
                    }

                    slot_ptr
                });

                // Write value outside closure — single memcpy from stack to slot
                // SAFETY: Slot is claimed from freelist, we have exclusive access
                unsafe {
                    (*slot_ptr).value =
                        std::mem::ManuallyDrop::new(std::mem::MaybeUninit::new(value));
                }
                slot_ptr
            }

            #[allow(clippy::unnecessary_wraps)]
            #[inline]
            fn try_alloc(
                value: Self::Item,
            ) -> Result<*mut SlotCell<$ty>, $crate::alloc::Full<$ty>> {
                Ok(Self::alloc(value))
            }

            unsafe fn dealloc(key: Key) {
                INNER.with(|inner| {
                    let (chunk_idx, local_idx) = inner.decode(key.index());
                    let chunk = inner.chunk(chunk_idx);
                    let chunk_inner = chunk.inner_ref();

                    let slot_ptr = chunk_inner.slots_ptr().add(local_idx as usize);

                    let free_head = chunk_inner.free_head.get();
                    let was_full = free_head.is_null();

                    // Write freelist link — overwrites value bytes (union semantics)
                    (*slot_ptr).next_free = free_head;
                    chunk_inner.free_head.set(slot_ptr);

                    if was_full {
                        inner.push_chunk_to_available(chunk_idx);
                    }
                });
            }

            #[inline]
            unsafe fn dealloc_ptr(slot_ptr: *mut SlotCell<$ty>) {
                INNER.with(|inner| {
                    // SAFETY: Caller guarantees slot_ptr is valid and within this slab
                    unsafe { inner.dealloc_ptr(slot_ptr) };
                });
            }

            #[inline]
            unsafe fn slot_cell(key: Key) -> *mut SlotCell<$ty> {
                INNER.with(|inner| {
                    let (chunk_idx, local_idx) = inner.decode(key.index());
                    let chunk = inner.chunk(chunk_idx);
                    chunk.inner_ref().slots_ptr().add(local_idx as usize)
                })
            }

            fn slot_index(slot_ptr: *mut SlotCell<$ty>) -> Key {
                INNER.with(|inner| {
                    // SAFETY: slot_ptr came from this slab's allocation
                    unsafe { inner.slot_to_index_global(slot_ptr) }
                })
            }
        }

        impl $crate::UnboundedAlloc for Allocator {}

        // Borrow/BorrowMut must be implemented here (not generically) because
        // the blanket `impl<T> Borrow<T> for T` conflicts with generic impls.
        impl std::borrow::Borrow<$ty> for $crate::alloc::Slot<Allocator> {
            #[inline]
            fn borrow(&self) -> &$ty {
                self
            }
        }

        impl std::borrow::BorrowMut<$ty> for $crate::alloc::Slot<Allocator> {
            #[inline]
            fn borrow_mut(&mut self) -> &mut $ty {
                self
            }
        }

        /// RAII handle to a slab-allocated value.
        ///
        /// Type alias for [`alloc::Slot<Allocator>`](crate::alloc::Slot).
        pub type Slot = $crate::alloc::Slot<Allocator>;
    };
}
