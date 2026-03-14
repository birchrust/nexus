//! Generic slab allocator trait and slot types.
//!
//! This module provides:
//! - [`Alloc`] - base trait for slot deallocation
//! - [`BoundedAlloc`] - trait for fixed-capacity allocators (can fail)
//! - [`UnboundedAlloc`] - trait for growable allocators (always succeeds)
//! - [`BoxSlot`] - 8-byte RAII handle generic over allocator
//! - [`RcSlot`] / [`WeakSlot`] - reference-counted handles

use std::borrow::{Borrow, BorrowMut};
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::ptr;

use crate::shared::{RawSlot, RcInner, SlotCell};

// =============================================================================
// Full<T>
// =============================================================================

/// Error returned when a bounded allocator is full.
///
/// Contains the value that could not be allocated, allowing recovery.
pub struct Full<T>(pub T);

impl<T> Full<T> {
    /// Consumes the error, returning the value that could not be allocated.
    #[inline]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Full(..)")
    }
}

impl<T> fmt::Display for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("allocator full")
    }
}

// =============================================================================
// Traits
// =============================================================================

/// Base trait for slab allocators - handles slot deallocation.
///
/// Each macro-generated allocator is a ZST that implements this trait.
/// All operations go through associated functions (no `&self`) since
/// the backing storage lives in a `thread_local!`.
///
/// # Safety
///
/// Implementors must guarantee:
/// - `free` correctly drops the stored item and returns the slot to the freelist.
///   For byte allocators (`AlignedBytes<N>` is `Copy`), `free` only does a
///   freelist return — the actual `T` value must be dropped by the caller
///   (e.g., `ByteBoxSlot::drop` calls `drop_in_place::<T>()` before `A::free`).
/// - `take` correctly moves the value out and returns the slot to the freelist
/// - All operations are single-threaded (TLS-backed)
pub unsafe trait Alloc: Sized + 'static {
    /// The type stored in each slot.
    type Item;

    /// Returns `true` if the allocator has been initialized.
    fn is_initialized() -> bool;

    /// Returns the total slot capacity.
    ///
    /// For bounded allocators this is fixed at init. For unbounded allocators
    /// this is the sum across all allocated chunks.
    fn capacity() -> usize;

    /// Drops the stored item and returns the slot to the freelist.
    ///
    /// For typed allocators, this drops `T` via `drop_in_place` then frees.
    /// For byte allocators (`AlignedBytes<N>` is `Copy`), this only does a
    /// freelist return — the caller must drop `T` before calling `free`.
    ///
    /// This is for manual memory management after calling `BoxSlot::into_slot()`.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this allocator
    /// - No references to the slot's value may exist
    /// - For byte allocators: the value must already have been dropped or moved out
    ///
    /// Note: Double-free is prevented at compile time (`RawSlot` is move-only).
    #[allow(clippy::needless_pass_by_value)] // consumes slot to prevent reuse
    unsafe fn free(slot: RawSlot<Self::Item>);

    /// Takes the value from a slot, returning it and deallocating the slot.
    ///
    /// This is for manual memory management after calling `BoxSlot::into_slot()`.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this allocator
    /// - No references to the slot's value may exist
    ///
    /// Note: Double-free is prevented at compile time (`RawSlot` is move-only).
    #[allow(clippy::needless_pass_by_value)] // consumes slot to prevent reuse
    unsafe fn take(slot: RawSlot<Self::Item>) -> Self::Item;
}

/// Trait for bounded (fixed-capacity) allocators.
///
/// Bounded allocators can fail when at capacity. Use [`try_alloc`](Self::try_alloc)
/// to handle capacity exhaustion.
pub trait BoundedAlloc: Alloc {
    /// Tries to allocate a slot and write the value.
    ///
    /// Returns `Err(Full(value))` if the allocator is full, giving the
    /// value back to the caller.
    fn try_alloc(value: Self::Item) -> Result<RawSlot<Self::Item>, Full<Self::Item>>;
}

/// Trait for unbounded (growable) allocators.
///
/// Unbounded allocators always succeed (grow as needed).
pub trait UnboundedAlloc: Alloc {
    /// Allocates a slot and writes the value.
    ///
    /// Always succeeds - grows the allocator if needed.
    fn alloc(value: Self::Item) -> RawSlot<Self::Item>;

    /// Ensures at least `count` chunks are allocated.
    ///
    /// No-op if the allocator already has `count` or more chunks.
    fn reserve_chunks(count: usize);

    /// Returns the number of allocated chunks.
    fn chunk_count() -> usize;
}

// =============================================================================
// BoxSlot<T, A>
// =============================================================================

/// RAII handle to a slab-allocated value, generic over allocator.
///
/// `BoxSlot<T, A>` is 8 bytes (one pointer).
///
/// This is the slot type generated by `bounded_allocator!` and
/// `unbounded_allocator!` macros via `type BoxSlot = alloc::BoxSlot<T, Allocator>`.
///
/// # Borrow Traits
///
/// `BoxSlot` implements [`Borrow<T>`] and [`BorrowMut<T>`], enabling use as
/// HashMap keys that borrow `T` for lookups.
///
/// # Thread Safety
///
/// `BoxSlot` is `!Send` and `!Sync` (via `PhantomData<*const ()>` inside the
/// marker). It must only be used from the thread that created it.
#[must_use = "dropping BoxSlot returns it to the allocator"]
pub struct BoxSlot<T, A: Alloc<Item = T>> {
    ptr: *mut SlotCell<T>,
    // PhantomData carries the allocator type AND makes BoxSlot !Send + !Sync
    // (*mut is !Send + !Sync, and PhantomData<A> ties the type)
    _marker: PhantomData<(A, *const ())>,
}

impl<T, A: Alloc<Item = T>> BoxSlot<T, A> {
    /// Leaks the slot permanently, returning an immutable reference.
    ///
    /// The value will never be dropped or deallocated. Use this for data
    /// that must live for the lifetime of the program.
    ///
    /// Returns a `LocalStatic<T>` which is `!Send + !Sync` and only supports
    /// immutable access via `Deref`.
    #[inline]
    pub fn leak(self) -> LocalStatic<T> {
        let slot_ptr = self.ptr;
        std::mem::forget(self);
        // SAFETY: Destructor won't run (forgot self).
        // The pointer is valid for 'static because slab storage is leaked.
        // Union field `value` is active because the slot is occupied.
        let value_ptr = unsafe { (*slot_ptr).value.as_ptr() };
        unsafe { LocalStatic::new(value_ptr) }
    }

    /// Converts to a raw slot for manual memory management.
    ///
    /// The slot is NOT deallocated. Caller must eventually:
    /// - Call `Allocator::free()` to drop and deallocate
    /// - Call `Allocator::take()` to extract value and deallocate
    /// - Wrap in another `BoxSlot` via `from_slot()`
    #[inline]
    pub fn into_slot(self) -> RawSlot<T> {
        let ptr = self.ptr;
        std::mem::forget(self);
        // SAFETY: ptr came from a valid allocation
        unsafe { RawSlot::from_ptr(ptr) }
    }

    /// Extracts the value from the slot, deallocating the slot.
    ///
    /// This is analogous to `Box::into_inner`.
    #[inline]
    pub fn into_inner(self) -> T {
        let ptr = self.ptr;
        std::mem::forget(self);

        // SAFETY: ptr came from a valid allocation, construct RawSlot for take
        let slot = unsafe { RawSlot::from_ptr(ptr) };
        // SAFETY: We owned the slot, no other references exist
        unsafe { A::take(slot) }
    }

    /// Replaces the value in the slot, returning the old value.
    #[inline]
    pub fn replace(&mut self, value: T) -> T {
        // SAFETY: We own the slot exclusively (&mut self), union field `value` is active
        unsafe {
            let val_ptr = (*(*self.ptr).value).as_mut_ptr();
            let old = ptr::read(val_ptr);
            ptr::write(val_ptr, value);
            old
        }
    }

    /// Returns a pinned reference to the value.
    ///
    /// Slab-allocated values have stable addresses — they never move while
    /// the slot is occupied. This makes `Pin` safe without any `Unpin` bound.
    ///
    /// Useful for async code that requires `Pin<&mut Self>` for polling futures.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut slot = order_alloc::BoxSlot::try_new(MyFuture::new())?;
    /// let pinned: Pin<&mut MyFuture> = slot.pin_mut();
    /// pinned.poll(cx);
    /// ```
    #[inline]
    pub fn pin(&self) -> Pin<&T> {
        // SAFETY: Slab values have stable addresses — they don't move until
        // the slot is explicitly freed. The BoxSlot owns the slot, so the
        // value cannot be freed while this reference exists.
        unsafe { Pin::new_unchecked(&**self) }
    }

    /// Returns a pinned mutable reference to the value.
    ///
    /// Slab-allocated values have stable addresses — they never move while
    /// the slot is occupied. This makes `Pin` safe without any `Unpin` bound.
    ///
    /// Useful for async code that requires `Pin<&mut Self>` for polling futures.
    #[inline]
    pub fn pin_mut(&mut self) -> Pin<&mut T> {
        // SAFETY: Slab values have stable addresses — they don't move until
        // the slot is explicitly freed. The BoxSlot owns the slot exclusively
        // (&mut self), so the value cannot be freed or moved while this
        // mutable reference exists.
        unsafe { Pin::new_unchecked(&mut **self) }
    }

    /// Wraps a raw slot in an RAII handle.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from an allocator of type `A`
    /// - `slot` must not be wrapped in another `BoxSlot` or otherwise managed
    #[inline]
    pub unsafe fn from_slot(slot: RawSlot<T>) -> Self {
        BoxSlot {
            ptr: slot.into_ptr(),
            _marker: PhantomData,
        }
    }

    /// Returns a raw pointer to the underlying slot cell.
    ///
    /// The pointer is valid as long as the `BoxSlot` (or any handle derived
    /// from the same slab slot) is alive.
    #[inline]
    pub fn as_ptr(&self) -> *mut SlotCell<T> {
        self.ptr
    }

    /// Consumes the `BoxSlot` and returns a raw pointer to the slot cell.
    ///
    /// The slot is NOT deallocated. The caller takes ownership and must
    /// eventually:
    /// - Call [`from_raw`](Self::from_raw) to reconstruct the `BoxSlot`
    /// - Or call [`Alloc::free`] / [`Alloc::take`] on the underlying [`RawSlot`]
    #[inline]
    pub fn into_raw(self) -> *mut SlotCell<T> {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }

    /// Reconstructs a `BoxSlot` from a raw pointer.
    ///
    /// # Safety
    ///
    /// - `ptr` must point to a valid, occupied slot cell within an allocator
    ///   of type `A`
    /// - The caller must own the slot (no other `BoxSlot` wrapping it)
    #[inline]
    pub unsafe fn from_raw(ptr: *mut SlotCell<T>) -> Self {
        BoxSlot {
            ptr,
            _marker: PhantomData,
        }
    }
}

impl<T, A: UnboundedAlloc<Item = T>> BoxSlot<T, A> {
    /// Creates a new slot containing the given value.
    ///
    /// Always succeeds - grows the allocator if needed.
    ///
    /// Only available for unbounded allocators. For bounded allocators,
    /// use [`try_new`](Self::try_new).
    #[inline]
    pub fn new(value: T) -> Self {
        BoxSlot {
            ptr: A::alloc(value).into_ptr(),
            _marker: PhantomData,
        }
    }
}

impl<T, A: BoundedAlloc<Item = T>> BoxSlot<T, A> {
    /// Tries to create a new slot containing the given value.
    ///
    /// Returns `Err(Full(value))` if the allocator is at capacity,
    /// giving the value back to the caller.
    ///
    /// Only available for bounded allocators. For unbounded allocators,
    /// use [`new`](Self::new) directly - it never fails.
    #[inline]
    pub fn try_new(value: T) -> Result<Self, Full<T>> {
        Ok(BoxSlot {
            ptr: A::try_alloc(value)?.into_ptr(),
            _marker: PhantomData,
        })
    }
}

// =============================================================================
// Trait Implementations for BoxSlot
// =============================================================================

impl<T, A: Alloc<Item = T>> Deref for BoxSlot<T, A> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: BoxSlot was created from a valid, occupied SlotCell.
        // Union field `value` is active because the slot is occupied.
        unsafe { (*self.ptr).value.assume_init_ref() }
    }
}

impl<T, A: Alloc<Item = T>> DerefMut for BoxSlot<T, A> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: We have &mut self, guaranteeing exclusive access.
        // Union field `value` is active because the slot is occupied.
        unsafe { (*(*self.ptr).value).assume_init_mut() }
    }
}

impl<T, A: Alloc<Item = T>> AsRef<T> for BoxSlot<T, A> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T, A: Alloc<Item = T>> AsMut<T> for BoxSlot<T, A> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T, A: Alloc<Item = T>> Borrow<T> for BoxSlot<T, A> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T, A: Alloc<Item = T>> BorrowMut<T> for BoxSlot<T, A> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

impl<T, A: Alloc<Item = T>> Drop for BoxSlot<T, A> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: We own the slot, construct RawSlot for A::free
        let slot = unsafe { RawSlot::from_ptr(self.ptr) };
        // SAFETY: We own the slot, no other references exist
        unsafe { A::free(slot) };
    }
}

impl<T: fmt::Debug, A: Alloc<Item = T>> fmt::Debug for BoxSlot<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxSlot").field("value", &**self).finish()
    }
}

// =============================================================================
// LocalStatic
// =============================================================================

/// A `'static` reference to a thread-local slab-allocated value.
///
/// Returned by [`BoxSlot::leak()`]. The reference is valid for the lifetime of
/// the program, but cannot be sent to other threads because the backing slab
/// is thread-local.
///
/// Once leaked, the slot is permanently occupied — there is no way to reclaim it.
#[repr(transparent)]
pub struct LocalStatic<T: ?Sized> {
    ptr: *const T,
    _marker: PhantomData<*const ()>, // !Send + !Sync
}

impl<T: ?Sized> LocalStatic<T> {
    /// Creates a new `LocalStatic` from a raw pointer.
    ///
    /// # Safety
    ///
    /// The pointer must point to a valid, permanently-leaked value in a
    /// thread-local slab.
    #[inline]
    pub(crate) unsafe fn new(ptr: *const T) -> Self {
        LocalStatic {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Returns a raw pointer to the value.
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.ptr
    }

    /// Returns a pinned reference to the value.
    ///
    /// Leaked slab values have stable addresses — they never move for the
    /// lifetime of the program. This makes `Pin` safe without any `Unpin` bound.
    #[inline]
    pub fn pin(&self) -> Pin<&T> {
        // SAFETY: Leaked values have stable addresses forever.
        unsafe { Pin::new_unchecked(&**self) }
    }
}

impl<T: ?Sized> Deref for LocalStatic<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: ptr came from a leaked BoxSlot, value is alive forever,
        // and we're on the same thread (enforced by !Send)
        unsafe { &*self.ptr }
    }
}

impl<T: ?Sized> AsRef<T> for LocalStatic<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: ?Sized> Clone for LocalStatic<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for LocalStatic<T> {}

impl<T: fmt::Debug + ?Sized> fmt::Debug for LocalStatic<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LocalStatic").field(&self.as_ref()).finish()
    }
}

// =============================================================================
// RcSlot<T, A>
// =============================================================================

/// Reference-counted handle to a slab-allocated value.
///
/// `RcSlot` is a cloneable, RAII handle backed by the existing slab allocator.
/// Cloning bumps the strong count. Dropping decrements it; when the last strong
/// reference drops, the value is dropped. The slab slot is freed when both
/// strong and weak counts reach zero.
///
/// 8 bytes — same as `BoxSlot`.
///
/// # Thread Safety
///
/// `RcSlot` is `!Send` and `!Sync` (same as `BoxSlot`). All access must be from
/// the thread that created the allocator.
#[must_use = "dropping RcSlot decrements the strong count"]
pub struct RcSlot<T, A: Alloc<Item = RcInner<T>>> {
    inner: ManuallyDrop<BoxSlot<RcInner<T>, A>>,
    _phantom: PhantomData<T>,
}

impl<T, A: UnboundedAlloc<Item = RcInner<T>>> RcSlot<T, A> {
    /// Creates a new `RcSlot` containing the given value.
    ///
    /// Always succeeds - grows the allocator if needed.
    ///
    /// Only available for unbounded allocators. For bounded allocators,
    /// use [`try_new`](Self::try_new).
    #[inline]
    pub fn new(value: T) -> Self {
        RcSlot {
            inner: ManuallyDrop::new(BoxSlot::new(RcInner::new(value))),
            _phantom: PhantomData,
        }
    }
}

impl<T, A: Alloc<Item = RcInner<T>>> RcSlot<T, A> {
    /// Creates a weak reference to the same slab slot.
    #[inline]
    pub fn downgrade(&self) -> WeakSlot<T, A> {
        let rc_inner: &RcInner<T> = &self.inner;
        let new_weak = rc_inner.weak().checked_add(1).expect("weak count overflow");
        rc_inner.set_weak(new_weak);
        // SAFETY: We hold a strong ref, slot is alive. Duplicate the pointer.
        let weak_slot = unsafe { BoxSlot::from_raw(self.inner.as_ptr()) };
        WeakSlot {
            inner: ManuallyDrop::new(weak_slot),
            _phantom: PhantomData,
        }
    }

    /// Returns the strong reference count.
    #[inline]
    pub fn strong_count(&self) -> u32 {
        let rc_inner: &RcInner<T> = &self.inner;
        rc_inner.strong()
    }

    /// Returns the weak reference count (excludes the implicit weak).
    #[inline]
    pub fn weak_count(&self) -> u32 {
        let rc_inner: &RcInner<T> = &self.inner;
        rc_inner.weak().saturating_sub(1)
    }

    /// Returns a pinned reference to the value.
    ///
    /// Slab-allocated values have stable addresses — they never move while
    /// the slot is occupied. This makes `Pin` safe without any `Unpin` bound.
    #[inline]
    pub fn pin(&self) -> Pin<&T> {
        // SAFETY: Slab values have stable addresses. The RcSlot keeps the
        // value alive, so the reference is valid.
        unsafe { Pin::new_unchecked(&**self) }
    }

    /// Returns a pinned mutable reference if this is the only reference.
    ///
    /// Returns `None` if there are other strong or weak references.
    ///
    /// Slab-allocated values have stable addresses — they never move while
    /// the slot is occupied. This makes `Pin` safe without any `Unpin` bound.
    #[inline]
    pub fn pin_get_mut(&mut self) -> Option<Pin<&mut T>> {
        self.get_mut().map(|r| {
            // SAFETY: Slab values have stable addresses. We verified exclusive
            // access via get_mut().
            unsafe { Pin::new_unchecked(r) }
        })
    }

    /// Returns a mutable reference if this is the only reference.
    ///
    /// Returns `None` if there are other strong or weak references.
    #[inline]
    pub fn get_mut(&mut self) -> Option<&mut T> {
        // Need strong == 1 AND weak == 0 (no outstanding weaks that could upgrade)
        if self.strong_count() == 1 && self.weak_count() == 0 {
            // SAFETY: We verified exclusive access
            Some(unsafe { self.get_mut_unchecked() })
        } else {
            None
        }
    }

    /// Returns a mutable reference to the value without checking the strong count.
    ///
    /// # Safety
    ///
    /// Caller must ensure this is the only `RcSlot` (strong_count == 1, weak_count == 0)
    /// and no `WeakSlot::upgrade` calls are concurrent.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_mut_unchecked(&self) -> &mut T {
        // SAFETY: Caller guarantees exclusive access.
        // Navigate through SlotCell union → RcInner → ManuallyDrop<T> → T
        let cell_ptr = self.inner.as_ptr();
        let rc_inner = unsafe { (*(*cell_ptr).value).assume_init_mut() };
        // SAFETY: value is live, caller guarantees exclusive access.
        // Dereference through ManuallyDrop to get &mut T.
        let md = unsafe { rc_inner.value_manual_drop_mut() };
        &mut *md
    }

    /// Converts to a raw slot for manual memory management.
    ///
    /// Returns `Some(Slot)` if this is the only reference (strong == 1, no weak refs).
    /// Returns `None` if other strong or weak references exist.
    ///
    /// The strong count is decremented but the value is NOT dropped.
    /// Caller takes ownership and must eventually free via the allocator.
    #[inline]
    pub fn into_slot(self) -> Option<RawSlot<RcInner<T>>> {
        let rc_inner: &RcInner<T> = &self.inner;

        // Must be only reference - strong == 1 and no external weaks (just implicit)
        if rc_inner.strong() != 1 || rc_inner.weak() != 1 {
            return None;
        }

        // Set counts to 0 - we're taking full ownership via raw Slot
        rc_inner.set_strong(0);
        rc_inner.set_weak(0);

        // Extract the raw slot pointer
        let slot_ptr = self.inner.as_ptr();

        // Don't run Drop (which would try to free)
        std::mem::forget(self);

        // SAFETY: We verified we're the only reference, slot_ptr is valid
        Some(unsafe { RawSlot::from_ptr(slot_ptr) })
    }

    /// Converts to a raw slot without checking refcounts.
    ///
    /// Caller takes full ownership of the slot. Refcounts are NOT modified —
    /// the caller is responsible for ensuring no other references exist or
    /// for handling the consequences.
    ///
    /// # Safety
    ///
    /// - Caller takes ownership of the slot and the value within
    /// - If other strong references exist, they will see stale refcounts
    ///   and may double-free or access dropped memory
    /// - If weak references exist, they will fail to upgrade (this is safe)
    ///   but may attempt deallocation based on stale counts
    #[inline]
    pub unsafe fn into_slot_unchecked(self) -> RawSlot<RcInner<T>> {
        // DON'T touch refcounts - caller takes full ownership
        // Any other refs will see stale counts, but that's caller's problem

        // Extract the raw slot pointer
        let slot_ptr = self.inner.as_ptr();

        // Don't run Drop
        std::mem::forget(self);

        unsafe { RawSlot::from_ptr(slot_ptr) }
    }

    // =========================================================================
    // Raw pointer API (mirrors std::rc::Rc)
    // =========================================================================

    /// Returns a raw pointer to the underlying slot cell.
    ///
    /// The pointer is valid as long as any strong reference exists.
    #[inline]
    pub fn as_ptr(&self) -> *mut SlotCell<RcInner<T>> {
        self.inner.as_ptr()
    }

    /// Consumes the `RcSlot` without decrementing the strong count.
    ///
    /// The caller takes responsibility for the strong count and must
    /// eventually call [`from_raw`](Self::from_raw) (to reconstruct and
    /// drop) or [`decrement_strong_count`](Self::decrement_strong_count).
    #[inline]
    pub fn into_raw(self) -> *mut SlotCell<RcInner<T>> {
        let ptr = self.inner.as_ptr();
        std::mem::forget(self);
        ptr
    }

    /// Reconstructs an `RcSlot` from a raw pointer without incrementing
    /// the strong count.
    ///
    /// # Safety
    ///
    /// - `ptr` must point to a valid, occupied `SlotCell<RcInner<T>>` within
    ///   an allocator of type `A`
    /// - The caller must own a strong count for this handle (e.g., obtained
    ///   via [`into_raw`](Self::into_raw) or
    ///   [`increment_strong_count`](Self::increment_strong_count))
    #[inline]
    pub unsafe fn from_raw(ptr: *mut SlotCell<RcInner<T>>) -> Self {
        RcSlot {
            inner: ManuallyDrop::new(unsafe { BoxSlot::from_raw(ptr) }),
            _phantom: PhantomData,
        }
    }

    /// Increments the strong count via a raw pointer.
    ///
    /// Use this when a data structure needs to acquire an additional strong
    /// reference from a raw pointer without holding an `RcSlot`.
    ///
    /// # Safety
    ///
    /// - `ptr` must point to a live `RcInner<T>` (strong > 0)
    #[inline]
    pub unsafe fn increment_strong_count(ptr: *mut SlotCell<RcInner<T>>) {
        // SAFETY: Caller guarantees ptr points to a live RcInner
        let rc_inner = unsafe { (*ptr).value.assume_init_ref() };
        let strong = rc_inner.strong();
        rc_inner.set_strong(strong + 1);
    }

    /// Decrements the strong count via a raw pointer.
    ///
    /// If the strong count reaches zero, the value is dropped. If both
    /// strong and weak counts reach zero, the slab slot is freed.
    ///
    /// # Safety
    ///
    /// - `ptr` must point to a valid `RcInner<T>`
    /// - The caller must own a strong count to decrement
    /// - After this call, `ptr` may be invalid if the slot was freed
    #[inline]
    pub unsafe fn decrement_strong_count(ptr: *mut SlotCell<RcInner<T>>) {
        // Reconstruct and drop — reuses existing Drop logic
        drop(unsafe { Self::from_raw(ptr) });
    }
}

impl<T, A: BoundedAlloc<Item = RcInner<T>>> RcSlot<T, A> {
    /// Tries to create a new `RcSlot` containing the given value.
    ///
    /// Returns `Err(Full(value))` if the allocator is at capacity.
    ///
    /// Only available for bounded allocators. For unbounded allocators,
    /// use [`new`](Self::new) directly - it never fails.
    #[inline]
    pub fn try_new(value: T) -> Result<Self, Full<T>> {
        match BoxSlot::try_new(RcInner::new(value)) {
            Ok(slot) => Ok(RcSlot {
                inner: ManuallyDrop::new(slot),
                _phantom: PhantomData,
            }),
            Err(full) => Err(Full(full.into_inner().into_value())),
        }
    }
}

impl<T: Clone, A: UnboundedAlloc<Item = RcInner<T>>> RcSlot<T, A> {
    /// Makes a mutable reference to the value, cloning if necessary.
    ///
    /// If this is the only reference (strong == 1, weak == 0), returns a
    /// mutable reference directly. Otherwise, clones the value into a new
    /// slot and returns a mutable reference to the clone.
    ///
    /// Always succeeds - grows the allocator if needed.
    ///
    /// Only available for unbounded allocators. For bounded allocators,
    /// use [`try_make_mut`](Self::try_make_mut).
    #[inline]
    pub fn make_mut(&mut self) -> &mut T {
        if self.strong_count() != 1 || self.weak_count() != 0 {
            // Clone into new slot, replace self
            *self = Self::new((**self).clone());
        }
        // SAFETY: Now we're the only reference
        unsafe { self.get_mut_unchecked() }
    }
}

impl<T: Clone, A: BoundedAlloc<Item = RcInner<T>>> RcSlot<T, A> {
    /// Tries to make a mutable reference to the value, cloning if necessary.
    ///
    /// If this is the only reference (strong == 1, weak == 0), returns a
    /// mutable reference directly. Otherwise, attempts to clone the value
    /// into a new slot.
    ///
    /// Returns `Err(Full)` if allocation fails.
    ///
    /// Only available for bounded allocators. For unbounded allocators,
    /// use [`make_mut`](Self::make_mut) directly - it never fails.
    #[inline]
    pub fn try_make_mut(&mut self) -> Result<&mut T, Full<()>> {
        if self.strong_count() != 1 || self.weak_count() != 0 {
            // Clone into new slot, replace self
            match Self::try_new((**self).clone()) {
                Ok(new_slot) => *self = new_slot,
                Err(_) => return Err(Full(())),
            }
        }
        // SAFETY: Now we're the only reference
        Ok(unsafe { self.get_mut_unchecked() })
    }
}

impl<T, A: Alloc<Item = RcInner<T>>> Clone for RcSlot<T, A> {
    #[inline]
    fn clone(&self) -> Self {
        let rc_inner: &RcInner<T> = &self.inner;
        let new_strong = rc_inner
            .strong()
            .checked_add(1)
            .expect("RcSlot strong count overflow");
        rc_inner.set_strong(new_strong);
        // SAFETY: We hold a strong ref, slot is alive
        let cloned_slot = unsafe { BoxSlot::from_raw(self.inner.as_ptr()) };
        RcSlot {
            inner: ManuallyDrop::new(cloned_slot),
            _phantom: PhantomData,
        }
    }
}

impl<T, A: Alloc<Item = RcInner<T>>> Drop for RcSlot<T, A> {
    #[inline]
    fn drop(&mut self) {
        // All refcount access goes through raw pointers to avoid Stacked
        // Borrows invalidation when we take &mut to drop the value.
        let cell_ptr = self.inner.as_ptr();

        // SAFETY: Slot is alive, union field `value` is active
        let strong = unsafe { (*cell_ptr).value.assume_init_ref().strong() };
        if strong > 1 {
            // SAFETY: same as above
            unsafe { (*cell_ptr).value.assume_init_ref().set_strong(strong - 1) };
            return;
        }

        // Last strong reference — drop the value
        // SAFETY: same as above
        unsafe { (*cell_ptr).value.assume_init_ref().set_strong(0) };

        // SAFETY: We are the last strong ref, value is live. We need &mut
        // to drop the ManuallyDrop<T> inside RcInner.
        unsafe {
            let rc_inner_mut = (*(*cell_ptr).value).assume_init_mut();
            ManuallyDrop::drop(rc_inner_mut.value_manual_drop_mut());
        }

        // Re-derive shared ref after the mutable drop above
        // SAFETY: RcInner is still valid memory (Cell<u32> fields are Copy,
        // ManuallyDrop<T> is dropped but the storage is still there)
        let weak = unsafe { (*cell_ptr).value.assume_init_ref().weak() };
        if weak == 1 {
            // No outstanding weaks — free the slot.
            // SAFETY: Value is dropped. Slot's drop_in_place on RcInner is
            // a no-op (ManuallyDrop<T> already dropped, Cell<u32> is Copy).
            // BoxSlot's Drop will call A::free() to return slot to freelist.
            unsafe { ManuallyDrop::drop(&mut self.inner) };
        } else {
            // SAFETY: same as weak read above
            unsafe { (*cell_ptr).value.assume_init_ref().set_weak(weak - 1) };
            // Zombie: T dropped, weak refs still hold the slot alive
        }
    }
}

impl<T, A: Alloc<Item = RcInner<T>>> Deref for RcSlot<T, A> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        let rc_inner: &RcInner<T> = &self.inner;
        rc_inner.value()
    }
}

impl<T, A: Alloc<Item = RcInner<T>>> AsRef<T> for RcSlot<T, A> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: fmt::Debug, A: Alloc<Item = RcInner<T>>> fmt::Debug for RcSlot<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RcSlot")
            .field("strong", &self.strong_count())
            .field("weak", &self.weak_count())
            .field("value", &**self)
            .finish()
    }
}

// =============================================================================
// WeakSlot<T, A>
// =============================================================================

/// Weak reference to a slab-allocated value.
///
/// Does not keep the value alive. Must [`upgrade`](Self::upgrade) to access
/// the value. Keeps the slab slot alive (for upgrade checks) until all weak
/// and strong references are dropped.
///
/// 8 bytes — same as `BoxSlot`.
pub struct WeakSlot<T, A: Alloc<Item = RcInner<T>>> {
    inner: ManuallyDrop<BoxSlot<RcInner<T>, A>>,
    _phantom: PhantomData<T>,
}

impl<T, A: Alloc<Item = RcInner<T>>> WeakSlot<T, A> {
    /// Attempts to upgrade to a strong reference.
    ///
    /// Returns `Some(RcSlot)` if the value is still alive (strong > 0),
    /// or `None` if the last strong reference has been dropped.
    #[inline]
    pub fn upgrade(&self) -> Option<RcSlot<T, A>> {
        let rc_inner: &RcInner<T> = &self.inner;
        let strong = rc_inner.strong();
        if strong == 0 {
            return None;
        }
        let new_strong = strong.checked_add(1).expect("RcSlot strong count overflow");
        rc_inner.set_strong(new_strong);
        // SAFETY: strong > 0 means slot is alive and value is valid
        let slot = unsafe { BoxSlot::from_raw(self.inner.as_ptr()) };
        Some(RcSlot {
            inner: ManuallyDrop::new(slot),
            _phantom: PhantomData,
        })
    }

    /// Returns the strong reference count.
    #[inline]
    pub fn strong_count(&self) -> u32 {
        let rc_inner: &RcInner<T> = &self.inner;
        rc_inner.strong()
    }

    /// Returns the weak reference count (excludes the implicit weak).
    #[inline]
    pub fn weak_count(&self) -> u32 {
        let rc_inner: &RcInner<T> = &self.inner;
        let weak = rc_inner.weak();
        // If strong > 0, subtract the implicit weak. If strong == 0,
        // the implicit weak was already decremented.
        if rc_inner.strong() > 0 {
            weak.saturating_sub(1)
        } else {
            weak
        }
    }
}

impl<T, A: Alloc<Item = RcInner<T>>> Clone for WeakSlot<T, A> {
    #[inline]
    fn clone(&self) -> Self {
        let rc_inner: &RcInner<T> = &self.inner;
        let new_weak = rc_inner
            .weak()
            .checked_add(1)
            .expect("WeakSlot weak count overflow");
        rc_inner.set_weak(new_weak);
        // SAFETY: We hold a weak ref, slot memory is alive
        let cloned_slot = unsafe { BoxSlot::from_raw(self.inner.as_ptr()) };
        WeakSlot {
            inner: ManuallyDrop::new(cloned_slot),
            _phantom: PhantomData,
        }
    }
}

impl<T, A: Alloc<Item = RcInner<T>>> Drop for WeakSlot<T, A> {
    #[inline]
    fn drop(&mut self) {
        let rc_inner: &RcInner<T> = &self.inner;
        let weak = rc_inner.weak();

        // Always decrement weak count
        rc_inner.set_weak(weak.saturating_sub(1));

        // Dealloc only if this was the last weak AND value already dropped (strong==0)
        if weak == 1 && rc_inner.strong() == 0 {
            // Zombie slot — value already dropped, dealloc the slot.
            // SAFETY: RcInner's ManuallyDrop<T> is already dropped.
            // BoxSlot's drop_in_place on RcInner is a no-op. Dealloc returns
            // the slot to the freelist.
            unsafe { ManuallyDrop::drop(&mut self.inner) };
        }
        // If strong > 0, strong holder's drop will handle dealloc.
        // If weak > 1, other weak refs still hold the slot alive.
    }
}

impl<T, A: Alloc<Item = RcInner<T>>> fmt::Debug for WeakSlot<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakSlot")
            .field("strong", &self.strong_count())
            .field("weak", &self.weak_count())
            .finish()
    }
}
