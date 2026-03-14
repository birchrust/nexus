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
//! // Use like Box (bounded allocators use try_new)
//! let slot = alloc::BoxSlot::try_new(Order { ... })?;
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
    initial_chunks: usize,
}

impl UnboundedBuilder {
    /// Default chunk size (4096 slots per chunk).
    pub const DEFAULT_CHUNK_SIZE: usize = 4096;

    /// Creates a new builder with default settings.
    #[inline]
    pub const fn new() -> Self {
        Self {
            chunk_size: Self::DEFAULT_CHUNK_SIZE,
            initial_chunks: 0,
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

    /// Sets the number of chunks to preallocate at initialization.
    ///
    /// By default, chunks are allocated on-demand. Setting this to N
    /// preallocates N chunks upfront, avoiding growth pauses during
    /// early operation.
    ///
    /// This is independent of `chunk_size` — e.g., `chunk_size(1024)`
    /// with `initial_chunks(4)` preallocates 4096 slots across 4 chunks.
    #[inline]
    pub const fn initial_chunks(mut self, count: usize) -> Self {
        self.initial_chunks = count;
        self
    }

    /// Returns the configured chunk size.
    #[inline]
    pub const fn get_chunk_size(&self) -> usize {
        self.chunk_size
    }

    /// Returns the configured initial chunk count.
    #[inline]
    pub const fn get_initial_chunks(&self) -> usize {
        self.initial_chunks
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
/// - `BoxSlot` - type alias for [`alloc::BoxSlot<T, Allocator>`](crate::alloc::BoxSlot)
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
///     // Use (bounded allocators use try_new since they can fail)
///     let slot = alloc::BoxSlot::try_new(Order { id: 1, price: 100.0 })?;
///     println!("Order price: {}", slot.price);  // Deref
///
///     Ok(())
/// }
/// ```
#[macro_export]
macro_rules! bounded_allocator {
    ($ty:ty) => {
        use $crate::SlotCell;
        use $crate::bounded::Slab as BoundedSlab;
        use $crate::macros::AlreadyInitialized;

        thread_local! {
            static SLAB: BoundedSlab<$ty> = const { BoundedSlab::new() };
        }

        /// Unit struct providing static methods for allocator lifecycle.
        #[derive(Clone, Copy)]
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
                SLAB.with(|slab| {
                    if slab.is_initialized() {
                        return Err(AlreadyInitialized);
                    }
                    slab.init(capacity);
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
        }

        // SAFETY: All operations use TLS freelist with proper union semantics.
        // Single-threaded by nature of TLS.
        unsafe impl $crate::Alloc for Allocator {
            type Item = $ty;

            #[inline]
            fn is_initialized() -> bool {
                SLAB.with(|slab| slab.is_initialized())
            }

            #[inline]
            fn capacity() -> usize {
                SLAB.with(|slab| slab.capacity())
            }

            #[inline]
            unsafe fn free(slot: $crate::RawSlot<$ty>) {
                let slot_ptr = slot.into_ptr();
                // Drop the value in place
                // SAFETY: Caller guarantees slot is valid and occupied
                std::ptr::drop_in_place((*(*slot_ptr).value).as_mut_ptr());
                // Return to freelist
                SLAB.with(|slab| {
                    // SAFETY: Value dropped, slot valid
                    slab.free_ptr(slot_ptr);
                });
            }

            #[inline]
            unsafe fn take(slot: $crate::RawSlot<$ty>) -> $ty {
                let slot_ptr = slot.into_ptr();
                // Move the value out
                // SAFETY: Caller guarantees slot is valid and occupied
                let value = std::ptr::read((*slot_ptr).value.as_ptr());
                // Return to freelist
                SLAB.with(|slab| {
                    // SAFETY: Value moved out, slot valid
                    slab.free_ptr(slot_ptr);
                });
                value
            }
        }

        impl $crate::BoundedAlloc for Allocator {
            #[inline]
            fn try_alloc(
                value: Self::Item,
            ) -> Result<$crate::RawSlot<$ty>, $crate::alloc::Full<$ty>> {
                // Claim slot from freelist — value stays on caller's stack, not captured
                let slot_ptr = SLAB.with(|slab| {
                    debug_assert!(slab.is_initialized(), "allocator not initialized");
                    slab.claim_ptr()
                });

                match slot_ptr {
                    Some(slot_ptr) => {
                        // Write value outside closure — enables placement new optimization
                        // SAFETY: Slot is claimed from freelist, we have exclusive access
                        unsafe {
                            (*slot_ptr).value =
                                std::mem::ManuallyDrop::new(std::mem::MaybeUninit::new(value));
                        }
                        // SAFETY: slot_ptr is valid and occupied
                        Ok(unsafe { $crate::RawSlot::from_ptr(slot_ptr) })
                    }
                    None => Err($crate::alloc::Full(value)),
                }
            }
        }

        /// RAII handle to a slab-allocated value.
        ///
        /// Type alias for [`alloc::BoxSlot<T, Allocator>`](crate::alloc::BoxSlot).
        pub type BoxSlot = $crate::alloc::BoxSlot<$ty, Allocator>;
    };
}

/// Generates a bounded (fixed-capacity) reference-counted slab allocator.
///
/// Wraps [`bounded_allocator!`] with `RcInner<$ty>` and adds `RcSlot`/`WeakSlot`
/// type aliases.
///
/// # Example
///
/// ```ignore
/// mod order_alloc {
///     nexus_slab::bounded_rc_allocator!(super::Order);
/// }
///
/// order_alloc::Allocator::builder().capacity(10_000).build()?;
/// let rc = order_alloc::RcSlot::try_new(Order { id: 1, price: 100.0 })?;
/// let weak = rc.downgrade();
/// let rc2 = rc.clone();
/// ```
#[macro_export]
macro_rules! bounded_rc_allocator {
    ($ty:ty) => {
        $crate::bounded_allocator!($crate::RcInner<$ty>);

        /// Strong reference-counted handle to a slab-allocated value.
        pub type RcSlot = $crate::alloc::RcSlot<$ty, Allocator>;
        /// Weak reference to a slab-allocated value.
        pub type WeakSlot = $crate::alloc::WeakSlot<$ty, Allocator>;
        /// Permanent reference to a leaked slab-allocated value.
        pub type LocalStatic = $crate::alloc::LocalStatic<$ty>;
    };
}

/// Generates an unbounded (growable) reference-counted slab allocator.
///
/// Wraps [`unbounded_allocator!`] with `RcInner<$ty>` and adds `RcSlot`/`WeakSlot`
/// type aliases.
///
/// # Example
///
/// ```ignore
/// mod order_alloc {
///     nexus_slab::unbounded_rc_allocator!(super::Order);
/// }
///
/// order_alloc::Allocator::builder().chunk_size(4096).build()?;
/// let rc = order_alloc::RcSlot::new(Order { id: 1, price: 100.0 });
/// ```
#[macro_export]
macro_rules! unbounded_rc_allocator {
    ($ty:ty) => {
        $crate::unbounded_allocator!($crate::RcInner<$ty>);

        /// Strong reference-counted handle to a slab-allocated value.
        pub type RcSlot = $crate::alloc::RcSlot<$ty, Allocator>;
        /// Weak reference to a slab-allocated value.
        pub type WeakSlot = $crate::alloc::WeakSlot<$ty, Allocator>;
        /// Permanent reference to a leaked slab-allocated value.
        pub type LocalStatic = $crate::alloc::LocalStatic<$ty>;
    };
}

/// Generates an unbounded (growable) slab allocator for a type.
///
/// This macro creates a module with:
/// - `Allocator` - unit struct with static methods for lifecycle management
/// - `BoxSlot` - type alias for [`alloc::BoxSlot<T, Allocator>`](crate::alloc::BoxSlot)
///
/// Unlike `bounded_allocator!`, `BoxSlot::new()` always succeeds by growing
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
///     let slot = alloc::BoxSlot::new(Quote { ... });
///
///     Ok(())
/// }
/// ```
#[macro_export]
macro_rules! unbounded_allocator {
    ($ty:ty) => {
        use $crate::SlotCell;
        use $crate::macros::AlreadyInitialized;
        use $crate::unbounded::Slab as UnboundedSlab;

        /// Default chunk size (4096 slots per chunk).
        const DEFAULT_CHUNK_SIZE: usize = 4096;

        thread_local! {
            static SLAB: UnboundedSlab<$ty> = const { UnboundedSlab::new() };
        }

        /// Unit struct providing static methods for allocator lifecycle.
        #[derive(Clone, Copy)]
        pub struct Allocator;

        /// Builder for configuring this allocator.
        pub struct Builder {
            chunk_size: usize,
            initial_chunks: usize,
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

            /// Sets the number of chunks to preallocate at initialization.
            ///
            /// By default, chunks are allocated on-demand. Setting this to N
            /// preallocates N chunks upfront, avoiding growth pauses during
            /// early operation.
            ///
            /// This is independent of `chunk_size` — e.g., `chunk_size(1024)`
            /// with `initial_chunks(4)` preallocates 4096 slots across 4 chunks.
            #[inline]
            pub fn initial_chunks(mut self, count: usize) -> Self {
                self.initial_chunks = count;
                self
            }

            /// Builds and initializes the allocator.
            ///
            /// # Errors
            ///
            /// Returns `AlreadyInitialized` if the allocator was already initialized.
            pub fn build(self) -> Result<(), AlreadyInitialized> {
                SLAB.with(|slab| {
                    if slab.is_initialized() {
                        return Err(AlreadyInitialized);
                    }
                    slab.init(self.chunk_size);
                    slab.reserve_chunks(self.initial_chunks);
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
                    initial_chunks: 0,
                }
            }
        }

        // SAFETY: All operations use TLS freelist with proper union semantics.
        // Single-threaded by nature of TLS.
        unsafe impl $crate::Alloc for Allocator {
            type Item = $ty;

            #[inline]
            fn is_initialized() -> bool {
                SLAB.with(|slab| slab.is_initialized())
            }

            #[inline]
            fn capacity() -> usize {
                SLAB.with(|slab| slab.capacity())
            }

            #[inline]
            unsafe fn free(slot: $crate::RawSlot<$ty>) {
                let slot_ptr = slot.into_ptr();
                // Drop the value in place
                // SAFETY: Caller guarantees slot is valid and occupied
                std::ptr::drop_in_place((*(*slot_ptr).value).as_mut_ptr());
                // Return to freelist
                SLAB.with(|slab| {
                    // SAFETY: Value dropped, slot valid
                    slab.free_ptr(slot_ptr);
                });
            }

            #[inline]
            unsafe fn take(slot: $crate::RawSlot<$ty>) -> $ty {
                let slot_ptr = slot.into_ptr();
                // Move the value out
                // SAFETY: Caller guarantees slot is valid and occupied
                let value = std::ptr::read((*slot_ptr).value.as_ptr());
                // Return to freelist
                SLAB.with(|slab| {
                    // SAFETY: Value moved out, slot valid
                    slab.free_ptr(slot_ptr);
                });
                value
            }
        }

        impl $crate::UnboundedAlloc for Allocator {
            #[inline]
            fn alloc(value: Self::Item) -> $crate::RawSlot<$ty> {
                // Claim slot from freelist — value stays on caller's stack, not captured
                // claim_ptr() returns (slot_ptr, chunk_idx), grows if needed
                let (slot_ptr, _chunk_idx) = SLAB.with(|slab| {
                    debug_assert!(slab.is_initialized(), "allocator not initialized");
                    slab.claim_ptr()
                });

                // Write value outside closure — enables placement new optimization
                // SAFETY: Slot is claimed from freelist, we have exclusive access
                unsafe {
                    (*slot_ptr).value =
                        std::mem::ManuallyDrop::new(std::mem::MaybeUninit::new(value));
                }
                // SAFETY: slot_ptr is valid and occupied
                unsafe { $crate::RawSlot::from_ptr(slot_ptr) }
            }

            #[inline]
            fn reserve_chunks(count: usize) {
                SLAB.with(|slab| slab.reserve_chunks(count));
            }

            #[inline]
            fn chunk_count() -> usize {
                SLAB.with(|slab| slab.chunk_count())
            }
        }

        /// RAII handle to a slab-allocated value.
        ///
        /// Type alias for [`alloc::BoxSlot<T, Allocator>`](crate::alloc::BoxSlot).
        pub type BoxSlot = $crate::alloc::BoxSlot<$ty, Allocator>;
    };
}

/// Generates a bounded (fixed-capacity) byte slab allocator.
///
/// Creates an allocator with fixed-size `N`-byte slots that can store any
/// type `T` where `size_of::<T>() <= N` and `align_of::<T>() <= 8`.
///
/// # Example
///
/// ```ignore
/// mod msg_alloc {
///     nexus_slab::bounded_byte_allocator!(64);
/// }
///
/// msg_alloc::Allocator::builder().capacity(10_000).build()?;
///
/// let order = msg_alloc::BoxSlot::<Order>::try_new(Order { id: 1 })?;
/// let cancel = msg_alloc::BoxSlot::<Cancel>::try_new(Cancel { id: 2 })?;
/// ```
#[macro_export]
macro_rules! bounded_byte_allocator {
    ($n:expr) => {
        use $crate::SlotCell;
        use $crate::bounded::Slab as BoundedSlab;
        use $crate::byte::AlignedBytes;
        use $crate::macros::AlreadyInitialized;

        const _: () = assert!(
            $n >= 8,
            "byte slab slot size must be at least 8 (pointer size)"
        );

        thread_local! {
            static SLAB: BoundedSlab<AlignedBytes<$n>> = const { BoundedSlab::new() };
        }

        /// Unit struct providing static methods for allocator lifecycle.
        #[derive(Clone, Copy)]
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
                SLAB.with(|slab| {
                    if slab.is_initialized() {
                        return Err(AlreadyInitialized);
                    }
                    slab.init(capacity);
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
        }

        // SAFETY: All operations use TLS freelist with proper union semantics.
        // Single-threaded by nature of TLS.
        unsafe impl $crate::Alloc for Allocator {
            type Item = AlignedBytes<$n>;

            #[inline]
            fn is_initialized() -> bool {
                SLAB.with(|slab| slab.is_initialized())
            }

            #[inline]
            fn capacity() -> usize {
                SLAB.with(|slab| slab.capacity())
            }

            #[inline]
            unsafe fn free(slot: $crate::RawSlot<AlignedBytes<$n>>) {
                let slot_ptr = slot.into_ptr();
                // AlignedBytes is Copy — no drop_in_place needed.
                // BoxSlot::drop already dropped the T value.
                SLAB.with(|slab| {
                    // SAFETY: Slot valid, value already handled by BoxSlot
                    slab.free_ptr(slot_ptr);
                });
            }

            #[inline]
            unsafe fn take(slot: $crate::RawSlot<AlignedBytes<$n>>) -> AlignedBytes<$n> {
                let slot_ptr = slot.into_ptr();
                // SAFETY: Caller guarantees slot is valid and occupied
                let value = std::ptr::read((*slot_ptr).value.as_ptr());
                SLAB.with(|slab| {
                    // SAFETY: Value moved out, slot valid
                    slab.free_ptr(slot_ptr);
                });
                value
            }
        }

        // SAFETY: claim_raw returns a valid, vacant slot from the TLS slab.
        // Single-threaded by nature of TLS.
        unsafe impl $crate::byte::BoundedByteAlloc for Allocator {
            #[inline]
            fn claim_raw() -> Option<*mut SlotCell<AlignedBytes<$n>>> {
                SLAB.with(|slab| {
                    debug_assert!(slab.is_initialized(), "allocator not initialized");
                    slab.claim_ptr()
                })
            }
        }

        /// RAII handle to a byte-slab-allocated value.
        ///
        /// Type alias for [`byte::BoxSlot<T, Allocator>`](crate::byte::BoxSlot).
        pub type BoxSlot<T: ?Sized> = $crate::byte::BoxSlot<T, Allocator>;
    };
}

/// Generates an unbounded (growable) byte slab allocator.
///
/// Creates an allocator with fixed-size `N`-byte slots that can store any
/// type `T` where `size_of::<T>() <= N` and `align_of::<T>() <= 8`.
///
/// Unlike [`bounded_byte_allocator!`], `BoxSlot::new()` always succeeds
/// by growing the allocator as needed.
///
/// # Example
///
/// ```ignore
/// mod msg_alloc {
///     nexus_slab::unbounded_byte_allocator!(64);
/// }
///
/// msg_alloc::Allocator::builder().build()?;
///
/// // Always succeeds — grows as needed
/// let order = msg_alloc::BoxSlot::<Order>::new(Order { id: 1 });
/// ```
#[macro_export]
macro_rules! unbounded_byte_allocator {
    ($n:expr) => {
        use $crate::SlotCell;
        use $crate::byte::AlignedBytes;
        use $crate::macros::AlreadyInitialized;
        use $crate::unbounded::Slab as UnboundedSlab;

        const _: () = assert!(
            $n >= 8,
            "byte slab slot size must be at least 8 (pointer size)"
        );

        /// Default chunk size (4096 slots per chunk).
        const DEFAULT_CHUNK_SIZE: usize = 4096;

        thread_local! {
            static SLAB: UnboundedSlab<AlignedBytes<$n>> = const { UnboundedSlab::new() };
        }

        /// Unit struct providing static methods for allocator lifecycle.
        #[derive(Clone, Copy)]
        pub struct Allocator;

        /// Builder for configuring this allocator.
        pub struct Builder {
            chunk_size: usize,
            initial_chunks: usize,
        }

        impl Builder {
            /// Sets the chunk size (optional, defaults to 4096).
            #[inline]
            pub fn chunk_size(mut self, size: usize) -> Self {
                self.chunk_size = size;
                self
            }

            /// Sets the number of chunks to preallocate at initialization.
            #[inline]
            pub fn initial_chunks(mut self, count: usize) -> Self {
                self.initial_chunks = count;
                self
            }

            /// Builds and initializes the allocator.
            ///
            /// # Errors
            ///
            /// Returns `AlreadyInitialized` if the allocator was already initialized.
            pub fn build(self) -> Result<(), AlreadyInitialized> {
                SLAB.with(|slab| {
                    if slab.is_initialized() {
                        return Err(AlreadyInitialized);
                    }
                    slab.init(self.chunk_size);
                    slab.reserve_chunks(self.initial_chunks);
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
                    initial_chunks: 0,
                }
            }
        }

        // SAFETY: All operations use TLS freelist with proper union semantics.
        // Single-threaded by nature of TLS.
        unsafe impl $crate::Alloc for Allocator {
            type Item = AlignedBytes<$n>;

            #[inline]
            fn is_initialized() -> bool {
                SLAB.with(|slab| slab.is_initialized())
            }

            #[inline]
            fn capacity() -> usize {
                SLAB.with(|slab| slab.capacity())
            }

            #[inline]
            unsafe fn free(slot: $crate::RawSlot<AlignedBytes<$n>>) {
                let slot_ptr = slot.into_ptr();
                // AlignedBytes is Copy — no drop_in_place needed.
                // BoxSlot::drop already dropped the T value.
                SLAB.with(|slab| {
                    // SAFETY: Slot valid, value already handled by BoxSlot
                    slab.free_ptr(slot_ptr);
                });
            }

            #[inline]
            unsafe fn take(slot: $crate::RawSlot<AlignedBytes<$n>>) -> AlignedBytes<$n> {
                let slot_ptr = slot.into_ptr();
                // SAFETY: Caller guarantees slot is valid and occupied
                let value = std::ptr::read((*slot_ptr).value.as_ptr());
                SLAB.with(|slab| {
                    // SAFETY: Value moved out, slot valid
                    slab.free_ptr(slot_ptr);
                });
                value
            }
        }

        // SAFETY: claim_raw always returns a valid, vacant slot.
        // Grows the allocator if needed. Single-threaded by nature of TLS.
        unsafe impl $crate::byte::UnboundedByteAlloc for Allocator {
            #[inline]
            fn claim_raw() -> *mut SlotCell<AlignedBytes<$n>> {
                SLAB.with(|slab| {
                    debug_assert!(slab.is_initialized(), "allocator not initialized");
                    let (ptr, _chunk_idx) = slab.claim_ptr();
                    ptr
                })
            }

            #[inline]
            fn reserve_chunks(count: usize) {
                SLAB.with(|slab| slab.reserve_chunks(count));
            }

            #[inline]
            fn chunk_count() -> usize {
                SLAB.with(|slab| slab.chunk_count())
            }
        }

        /// RAII handle to a byte-slab-allocated value.
        ///
        /// Type alias for [`byte::BoxSlot<T, Allocator>`](crate::byte::BoxSlot).
        pub type BoxSlot<T: ?Sized> = $crate::byte::BoxSlot<T, Allocator>;
    };
}
