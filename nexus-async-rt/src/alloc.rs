//! Task allocator abstraction.
//!
//! The [`TaskAlloc`] sealed trait abstracts over bounded (fixed-capacity)
//! and unbounded (growable) byte slab storage for tasks. Users choose
//! their allocator before constructing the runtime.
//!
//! The const generic `S` is the internal slab slot size (future bytes +
//! 24-byte task header). Use [`slot_size`](crate::slot_size) to compute:
//!
//! ```ignore
//! use nexus_async_rt::{slot_size, BoundedTaskAlloc, UnboundedTaskAlloc};
//!
//! // 256-byte future capacity → 280-byte internal slot
//! let alloc = BoundedTaskAlloc::<{ slot_size(256) }>::new(64);
//! let alloc = UnboundedTaskAlloc::<{ slot_size(256) }>::new(64);
//! ```
//!
//! For convenience, [`DefaultBoundedAlloc`] and [`DefaultUnboundedAlloc`]
//! use 256-byte future capacity (covers most IO tasks).

use crate::task::{Task, TASK_HEADER_SIZE};

mod sealed {
    pub trait Sealed {}
}

/// Sealed trait for task allocators. Cannot be implemented outside
/// this crate.
///
/// The `alloc_task` method takes `Task<F>` which is `pub(crate)` — this
/// is intentional. The trait is sealed, so only crate-internal code calls it.
/// Users don't need this trait — pick [`BoundedTaskAlloc`] or
/// [`UnboundedTaskAlloc`] and pass to [`RuntimeBuilder`](crate::RuntimeBuilder).
#[allow(private_interfaces)]
pub trait TaskAlloc: sealed::Sealed {
    /// Allocate a task via placement new. Returns raw pointer.
    fn alloc_task<F: std::future::Future<Output = ()> + 'static>(
        &self,
        task: Task<F>,
    ) -> *mut u8;

    /// Returns the maximum task capacity, or `None` if unbounded.
    fn max_capacity(&self) -> Option<usize>;

    /// Free a task slot.
    ///
    /// # Safety
    ///
    /// `ptr` must have been returned by `alloc_task` on this allocator.
    unsafe fn free(&self, ptr: *mut u8);
}

// =============================================================================
// Bounded
// =============================================================================

/// Fixed-capacity task allocator. Panics on spawn if full.
///
/// `S` is the internal slot size (use [`slot_size`](crate::slot_size)
/// to compute from your max future size).
pub struct BoundedTaskAlloc<const S: usize> {
    slab: nexus_slab::byte::bounded::Slab<S>,
    capacity: usize,
}

impl<const S: usize> BoundedTaskAlloc<S> {
    /// Create a bounded allocator with `capacity` task slots.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize) -> Self {
        // SAFETY: single-threaded use. Slot lifetimes managed by Executor.
        let slab = unsafe { nexus_slab::byte::bounded::Slab::with_capacity(capacity) };
        Self { slab, capacity }
    }

    /// Returns the fixed capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<const S: usize> sealed::Sealed for BoundedTaskAlloc<S> {}

#[allow(private_interfaces)]
impl<const S: usize> TaskAlloc for BoundedTaskAlloc<S> {
    fn alloc_task<F: std::future::Future<Output = ()> + 'static>(
        &self,
        task: Task<F>,
    ) -> *mut u8 {
        self.slab.alloc(task).into_raw()
    }

    fn max_capacity(&self) -> Option<usize> {
        Some(self.capacity)
    }

    unsafe fn free(&self, ptr: *mut u8) {
        let slot = unsafe { nexus_slab::byte::Slot::<u8>::from_raw(ptr) };
        self.slab.free(slot);
    }
}

// =============================================================================
// Unbounded
// =============================================================================

/// Growable task allocator. Never fails — allocates new chunks as needed.
///
/// `S` is the internal slot size (use [`slot_size`](crate::slot_size)).
pub struct UnboundedTaskAlloc<const S: usize> {
    slab: nexus_slab::byte::unbounded::Slab<S>,
}

impl<const S: usize> UnboundedTaskAlloc<S> {
    /// Create an unbounded allocator with `initial_chunk_capacity` slots
    /// in the first chunk.
    pub fn new(initial_chunk_capacity: usize) -> Self {
        // SAFETY: single-threaded use. Slot lifetimes managed by Executor.
        let slab = unsafe {
            nexus_slab::byte::unbounded::Slab::with_chunk_capacity(initial_chunk_capacity)
        };
        Self { slab }
    }
}

impl<const S: usize> sealed::Sealed for UnboundedTaskAlloc<S> {}

#[allow(private_interfaces)]
impl<const S: usize> TaskAlloc for UnboundedTaskAlloc<S> {
    fn alloc_task<F: std::future::Future<Output = ()> + 'static>(
        &self,
        task: Task<F>,
    ) -> *mut u8 {
        self.slab.alloc(task).into_raw()
    }

    fn max_capacity(&self) -> Option<usize> {
        None // unbounded — always has room
    }

    unsafe fn free(&self, ptr: *mut u8) {
        let slot = unsafe { nexus_slab::byte::Slot::<u8>::from_raw(ptr) };
        self.slab.free(slot);
    }
}

// =============================================================================
// Defaults (256-byte future capacity)
// =============================================================================

/// Bounded allocator with 256-byte future capacity (280-byte slots).
pub type DefaultBoundedAlloc = BoundedTaskAlloc<{ 256 + TASK_HEADER_SIZE }>;

/// Unbounded allocator with 256-byte future capacity (280-byte slots).
pub type DefaultUnboundedAlloc = UnboundedTaskAlloc<{ 256 + TASK_HEADER_SIZE }>;
