//! Byte slab types for type-erased allocation.
//!
//! This module provides:
//! - [`AlignedBytes`] — fixed-size byte storage with pointer alignment
//! - [`BoundedByteAlloc`] / [`UnboundedByteAlloc`] — traits for byte slab allocators
//! - [`BoxSlot`] — RAII handle for TLS byte allocators
//!   (8 bytes for `Sized` types, 16 bytes for `dyn Trait`)
//! - [`Slot`] — move-only handle for struct-owned byte slabs
//!   (8 bytes for `Sized` types, 16 bytes for `dyn Trait`)

use std::borrow::{Borrow, BorrowMut};
use std::fmt;
use std::marker::PhantomData;
use std::mem::{self, align_of, size_of};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::ptr;

use crate::alloc::{Alloc, Full, LocalStatic};
use crate::shared::{RawSlot, SlotCell};

// =============================================================================
// AlignedBytes
// =============================================================================

/// Fixed-size byte storage with pointer alignment.
///
/// Used as `SlotCell<AlignedBytes<N>>` in byte slab allocators. The 8-byte
/// alignment matches the `next_free` pointer in the `SlotCell` union and
/// covers all common types (up to `u64`, pointers, most structs).
///
/// Types requiring greater than 8-byte alignment (e.g., SIMD vectors)
/// cannot be stored in a byte slab.
///
/// `Copy` guarantees `drop_in_place` is a compile-time no-op.
#[derive(Clone, Copy)]
#[repr(C, align(8))]
pub struct AlignedBytes<const N: usize> {
    bytes: [u8; N],
}

// =============================================================================
// Traits
// =============================================================================

/// Trait for bounded byte slab allocators.
///
/// Provides raw slot claiming so [`BoxSlot`] can write `T` directly
/// into slot memory without constructing an intermediate `AlignedBytes`.
///
/// # Safety
///
/// Implementors must guarantee:
/// - `claim_raw` returns a valid, vacant slot from the TLS slab
/// - The returned pointer is exclusively owned by the caller
pub unsafe trait BoundedByteAlloc: Alloc {
    /// Claims a raw slot pointer from the freelist.
    ///
    /// Returns `None` if the allocator is full.
    fn claim_raw() -> Option<*mut SlotCell<Self::Item>>;
}

/// Trait for unbounded byte slab allocators.
///
/// Always succeeds — grows the allocator if needed.
///
/// # Safety
///
/// Same guarantees as [`BoundedByteAlloc`], plus the returned pointer
/// is always valid (allocator grows on demand).
pub unsafe trait UnboundedByteAlloc: Alloc {
    /// Claims a raw slot pointer, growing the allocator if needed.
    fn claim_raw() -> *mut SlotCell<Self::Item>;

    /// Ensures at least `count` chunks are allocated.
    fn reserve_chunks(count: usize);

    /// Returns the number of allocated chunks.
    fn chunk_count() -> usize;
}

// =============================================================================
// BoxSlot<T, A>
// =============================================================================

/// RAII handle to a byte-slab-allocated value, generic over allocator.
///
/// `BoxSlot<T, A>` stores a value of type `T` in a byte slab managed by
/// allocator `A`. The allocator manages [`AlignedBytes<N>`] storage, while
/// this handle provides typed access via `Deref<Target = T>` and correctly
/// drops `T` when the handle is dropped.
///
/// # Size
///
/// - 8 bytes for `Sized` types (thin pointer)
/// - 16 bytes for `dyn Trait` types (fat pointer = data ptr + vtable ptr)
///
/// # Thread Safety
///
/// `BoxSlot` is `!Send` and `!Sync`. It must only be used from the
/// thread that created it.
///
/// # Compile-Time Safety
///
/// [`try_new`](Self::try_new) and [`new`](Self::new) include `const`
/// assertions that verify:
/// - `size_of::<T>() <= N` — T fits in the slot
/// - `align_of::<T>() <= 8` — T alignment is compatible
///
/// Violations are compile errors, not runtime panics.
#[must_use = "dropping BoxSlot returns it to the allocator"]
pub struct BoxSlot<T: ?Sized, A: Alloc> {
    ptr: *mut T,
    _marker: PhantomData<(A, *const ())>,
}

// =============================================================================
// Sized-only constructors (bounded)
// =============================================================================

impl<T, A: BoundedByteAlloc> BoxSlot<T, A> {
    /// Tries to create a new slot containing the given value.
    ///
    /// Returns `Err(Full(value))` if the allocator is at capacity,
    /// giving the value back to the caller.
    ///
    /// # Compile-Time Checks
    ///
    /// Fails to compile if `T` is too large or too aligned for the slot.
    #[inline]
    pub fn try_new(value: T) -> Result<Self, Full<T>> {
        const {
            assert!(
                size_of::<T>() <= size_of::<A::Item>(),
                "T does not fit in byte slab slot"
            );
        };
        const {
            assert!(
                align_of::<T>() <= align_of::<A::Item>(),
                "T alignment exceeds slot alignment"
            );
        };

        match A::claim_raw() {
            Some(slot_ptr) => {
                // SAFETY: slot_ptr is a valid, vacant slot exclusively owned
                // by us. T fits within AlignedBytes<N> (const asserted above).
                // SlotCell is repr(C) union with fields at offset 0;
                // ManuallyDrop and MaybeUninit are transparent; AlignedBytes
                // is repr(C) with bytes at offset 0. So slot_ptr points to
                // where T's bytes go.
                unsafe {
                    write_and_zero_pad::<T, A>(slot_ptr, value);
                }
                Ok(BoxSlot {
                    ptr: slot_ptr as *mut T,
                    _marker: PhantomData,
                })
            }
            None => Err(Full(value)),
        }
    }

    /// Tries to create a slot containing `value`, returning a handle typed
    /// as `BoxSlot<U, A>` where `U: ?Sized`.
    ///
    /// The `coerce` function converts the concrete `*mut T` to a fat pointer
    /// `*mut U` (e.g., `|p| p as *mut dyn Trait`).
    ///
    /// # Compile-Time Checks
    ///
    /// Same as [`try_new`](Self::try_new).
    #[inline]
    pub fn try_new_as<U: ?Sized>(
        value: T,
        coerce: fn(*mut T) -> *mut U,
    ) -> Result<BoxSlot<U, A>, Full<T>> {
        match Self::try_new(value) {
            Ok(slot) => Ok(slot.unsize(coerce)),
            Err(full) => Err(full),
        }
    }
}

// =============================================================================
// Sized-only constructors (unbounded)
// =============================================================================

impl<T, A: UnboundedByteAlloc> BoxSlot<T, A> {
    /// Creates a new slot containing the given value.
    ///
    /// Always succeeds — grows the allocator if needed.
    ///
    /// # Compile-Time Checks
    ///
    /// Fails to compile if `T` is too large or too aligned for the slot.
    #[inline]
    pub fn new(value: T) -> Self {
        const {
            assert!(
                size_of::<T>() <= size_of::<A::Item>(),
                "T does not fit in byte slab slot"
            );
        };
        const {
            assert!(
                align_of::<T>() <= align_of::<A::Item>(),
                "T alignment exceeds slot alignment"
            );
        };

        let slot_ptr = A::claim_raw();
        // SAFETY: Same as try_new — slot_ptr is valid and exclusively ours.
        unsafe {
            write_and_zero_pad::<T, A>(slot_ptr, value);
        }
        BoxSlot {
            ptr: slot_ptr as *mut T,
            _marker: PhantomData,
        }
    }

    /// Creates a slot containing `value`, returning a handle typed as
    /// `BoxSlot<U, A>` where `U: ?Sized`.
    ///
    /// The `coerce` function converts the concrete `*mut T` to a fat pointer
    /// `*mut U` (e.g., `|p| p as *mut dyn Trait`).
    ///
    /// # Compile-Time Checks
    ///
    /// Same as [`new`](Self::new).
    #[inline]
    pub fn new_as<U: ?Sized>(value: T, coerce: fn(*mut T) -> *mut U) -> BoxSlot<U, A> {
        Self::new(value).unsize(coerce)
    }
}

// =============================================================================
// Sized-only methods
// =============================================================================

impl<T, A: Alloc> BoxSlot<T, A> {
    /// Extracts the value from the slot, deallocating the slot.
    #[inline]
    pub fn into_inner(self) -> T {
        // SAFETY: T is Sized, so self.ptr is a thin pointer whose address
        // is the start of the SlotCell. Read the value, reconstruct the
        // Slot for freeing.
        let data_ptr = self.ptr;
        mem::forget(self);
        let value = unsafe { ptr::read(data_ptr) };
        // SAFETY: data_ptr is the address of the SlotCell<A::Item>.
        // Reconstruct RawSlot to pass to A::free.
        let slot = unsafe { RawSlot::from_ptr(data_ptr as *mut SlotCell<A::Item>) };
        unsafe { A::free(slot) };
        value
    }

    /// Replaces the value in the slot, returning the old value.
    #[inline]
    pub fn replace(&mut self, value: T) -> T {
        // SAFETY: We own the slot exclusively (&mut self). T is at offset 0.
        unsafe {
            let old = ptr::read(self.ptr);
            ptr::write(self.ptr, value);
            old
        }
    }

    /// Converts this `BoxSlot<T, A>` into a `BoxSlot<U, A>` where
    /// `U: ?Sized`, using the given coercion function.
    ///
    /// This is the low-level API for unsizing. For convenience, use the
    /// [`box_dyn!`](crate::box_dyn) or
    /// [`try_box_dyn!`](crate::try_box_dyn) macros.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let sized: BoxSlot<MyHandler, A> = BoxSlot::new(handler);
    /// let dyn_slot: BoxSlot<dyn Handler<E>, A> = sized.unsize(|p| p as *mut dyn Handler<E>);
    /// ```
    #[inline]
    pub fn unsize<U: ?Sized>(self, coerce: fn(*mut T) -> *mut U) -> BoxSlot<U, A> {
        let thin_ptr = self.ptr;
        let fat_ptr = coerce(thin_ptr);
        // Verify the coercion didn't change the data pointer.
        // This is assert, not debug_assert — an incorrect coerce function
        // would cause UB in Drop (wrong data pointer → wrong slot freed).
        assert_eq!(
            fat_ptr as *const () as usize, thin_ptr as *const () as usize,
            "coerce function must not change the data pointer address"
        );
        mem::forget(self);
        BoxSlot {
            ptr: fat_ptr,
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// ?Sized methods
// =============================================================================

impl<T: ?Sized, A: Alloc> BoxSlot<T, A> {
    /// Leaks the slot permanently, returning an immutable reference.
    ///
    /// The value will never be dropped or deallocated.
    #[inline]
    pub fn leak(self) -> LocalStatic<T> {
        let ptr = self.ptr.cast_const();
        mem::forget(self);
        // SAFETY: Slot is permanently leaked. ptr points to a valid T.
        unsafe { LocalStatic::new(ptr) }
    }

    /// Returns a pinned reference to the value.
    ///
    /// Slab-allocated values have stable addresses — they never move while
    /// the slot is occupied.
    #[inline]
    pub fn pin(&self) -> Pin<&T> {
        // SAFETY: Slab values have stable addresses. The BoxSlot owns
        // the slot, so the value cannot be freed while this reference exists.
        unsafe { Pin::new_unchecked(&**self) }
    }

    /// Returns a pinned mutable reference to the value.
    ///
    /// Slab-allocated values have stable addresses — they never move while
    /// the slot is occupied.
    #[inline]
    pub fn pin_mut(&mut self) -> Pin<&mut T> {
        // SAFETY: Slab values have stable addresses. We have exclusive
        // access (&mut self).
        unsafe { Pin::new_unchecked(&mut **self) }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl<T: ?Sized, A: Alloc> Deref for BoxSlot<T, A> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: self.ptr points to a valid, occupied T value within the
        // slab. For Sized T this is a thin pointer cast; for dyn Trait this
        // is a fat pointer that carries the vtable.
        unsafe { &*self.ptr }
    }
}

impl<T: ?Sized, A: Alloc> DerefMut for BoxSlot<T, A> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: We have &mut self, guaranteeing exclusive access.
        unsafe { &mut *self.ptr }
    }
}

impl<T: ?Sized, A: Alloc> AsRef<T> for BoxSlot<T, A> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: ?Sized, A: Alloc> AsMut<T> for BoxSlot<T, A> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: ?Sized, A: Alloc> Borrow<T> for BoxSlot<T, A> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T: ?Sized, A: Alloc> BorrowMut<T> for BoxSlot<T, A> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: ?Sized, A: Alloc> Drop for BoxSlot<T, A> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: We own the slot. Drop T first, then free the slot.
        // For dyn Trait, drop_in_place dispatches through the vtable.
        // Extract the data pointer (drops vtable for fat ptrs) to
        // reconstruct the Slot for A::free.
        //
        // A::free MUST NOT call drop_in_place — it only returns the slot
        // to the freelist. We handle the drop here. This is guaranteed by
        // the byte Alloc trait's free() contract (see alloc.rs).
        unsafe {
            ptr::drop_in_place(self.ptr);
            let data_ptr = self.ptr as *mut () as *mut SlotCell<A::Item>;
            let slot = RawSlot::from_ptr(data_ptr);
            A::free(slot);
        }
    }
}

impl<T: fmt::Debug + ?Sized, A: Alloc> fmt::Debug for BoxSlot<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxSlot").field("value", &&**self).finish()
    }
}

// =============================================================================
// Slot<T: ?Sized>
// =============================================================================

/// Move-only handle to a value stored in a byte slab.
///
/// Unlike [`BoxSlot`] (TLS allocator), `Slot` is for struct-owned slabs
/// (`bounded::Slab<AlignedBytes<N>>` or `unbounded::Slab<AlignedBytes<N>>`).
/// It does NOT auto-free on drop — the caller must return it to the slab via
/// [`remove`](crate::bounded::Slab::remove),
/// [`take_value`](crate::bounded::Slab::take_value), or
/// [`reclaim`](crate::bounded::Slab::reclaim).
///
/// # Size
///
/// - 8 bytes for `Sized` types (thin pointer)
/// - 16 bytes for `dyn Trait` types (fat pointer)
///
/// # Thread Safety
///
/// `Slot` is `!Send` and `!Sync`.
#[must_use = "slot must be freed via slab.remove() or slab.take_value()"]
pub struct Slot<T: ?Sized> {
    ptr: *mut T,
    _marker: PhantomData<*const ()>, // !Send + !Sync
}

// =============================================================================
// Slot — Sized-only methods
// =============================================================================

impl<T> Slot<T> {
    /// Unsizes this handle (e.g., concrete → dyn Trait).
    ///
    /// The `coerce` function converts the concrete `*mut T` to a fat pointer
    /// `*mut U` (e.g., `|p| p as *mut dyn Trait`).
    #[inline]
    pub fn unsize<U: ?Sized>(self, coerce: fn(*mut T) -> *mut U) -> Slot<U> {
        let thin_ptr = self.ptr;
        let fat_ptr = coerce(thin_ptr);
        // assert, not debug_assert — an incorrect coerce function would cause
        // UB when the slab frees the wrong slot.
        assert_eq!(
            fat_ptr as *const () as usize, thin_ptr as *const () as usize,
            "coerce function must not change the data pointer address"
        );
        mem::forget(self);
        Slot {
            ptr: fat_ptr,
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Slot — ?Sized methods
// =============================================================================

impl<T: ?Sized> Slot<T> {
    /// Creates a `Slot` from a raw pointer.
    ///
    /// # Safety
    ///
    /// - `ptr` must point to a valid, live value within a byte slab
    /// - The caller transfers ownership to the `Slot`
    #[inline]
    pub(crate) unsafe fn from_raw(ptr: *mut T) -> Self {
        Slot {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Extracts the raw pointer, consuming the `Slot` without running
    /// the debug-mode leak detector.
    #[inline]
    pub(crate) fn into_raw(self) -> *mut T {
        let ptr = self.ptr;
        mem::forget(self);
        ptr
    }

    /// Returns a pinned reference to the value.
    ///
    /// Slab-allocated values have stable addresses — they never move while
    /// the slot is occupied.
    #[inline]
    pub fn pin(&self) -> Pin<&T> {
        // SAFETY: Slab values have stable addresses. The Slot owns
        // the value, so it cannot be freed while this reference exists.
        unsafe { Pin::new_unchecked(&**self) }
    }

    /// Returns a pinned mutable reference to the value.
    ///
    /// Slab-allocated values have stable addresses — they never move while
    /// the slot is occupied.
    #[inline]
    pub fn pin_mut(&mut self) -> Pin<&mut T> {
        // SAFETY: Slab values have stable addresses. We have exclusive
        // access (&mut self).
        unsafe { Pin::new_unchecked(&mut **self) }
    }
}

// =============================================================================
// Slot — Trait Implementations
// =============================================================================

impl<T: ?Sized> Deref for Slot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: self.ptr points to a valid, occupied T value within the
        // slab. For Sized T this is a thin pointer; for dyn Trait this is
        // a fat pointer carrying the vtable.
        unsafe { &*self.ptr }
    }
}

impl<T: ?Sized> DerefMut for Slot<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: We have &mut self, guaranteeing exclusive access.
        unsafe { &mut *self.ptr }
    }
}

impl<T: ?Sized> AsRef<T> for Slot<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: ?Sized> AsMut<T> for Slot<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: ?Sized> Borrow<T> for Slot<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T: ?Sized> BorrowMut<T> for Slot<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot").field("value", &&**self).finish()
    }
}

#[cfg(debug_assertions)]
impl<T: ?Sized> Drop for Slot<T> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // During unwinding: log but don't abort. Leak is lesser evil than abort.
            eprintln!(
                "byte::Slot<{}> leaked during panic unwind (was not freed)",
                std::any::type_name::<T>()
            );
        } else {
            panic!(
                "byte::Slot<{}> dropped without being freed — \
                 call slab.remove() or slab.take_value()",
                std::any::type_name::<T>()
            );
        }
    }
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Writes `value` into a slot and zeroes trailing bytes.
///
/// # Safety
///
/// - `slot_ptr` must be a valid, exclusively-owned, vacant slot
/// - `T` must fit within `A::Item` (caller must const-assert)
#[inline]
unsafe fn write_and_zero_pad<T, A: Alloc>(slot_ptr: *mut SlotCell<A::Item>, value: T) {
    // SAFETY: Caller guarantees slot_ptr is valid and exclusively owned.
    // T fits within A::Item (caller must const-assert).
    unsafe {
        ptr::write(slot_ptr as *mut T, value);
        // Ensures the full AlignedBytes<N> is deterministically initialized.
        // When size_of::<T>() == size_of::<A::Item>(), the compiler
        // eliminates this entirely.
        let t_size = size_of::<T>();
        let slot_size = size_of::<A::Item>();
        if t_size < slot_size {
            ptr::write_bytes((slot_ptr as *mut u8).add(t_size), 0, slot_size - t_size);
        }
    }
}

// =============================================================================
// Convenience macros
// =============================================================================

/// Creates a `BoxSlot<dyn Trait, A>` from a concrete value.
///
/// For unbounded byte allocators (always succeeds).
///
/// # Example
///
/// ```ignore
/// let handler = nexus_slab::box_dyn!(
///     msg_alloc::Allocator, dyn Handler<E>, my_handler
/// );
/// ```
#[macro_export]
macro_rules! box_dyn {
    ($alloc:ty, $dyn_ty:ty, $value:expr) => {{ <$crate::byte::BoxSlot<_, $alloc>>::new_as($value, |__p| __p as *mut $dyn_ty) }};
}

/// Creates a `BoxSlot<dyn Trait, A>` from a concrete value.
///
/// For bounded byte allocators (returns `Result`).
///
/// # Example
///
/// ```ignore
/// let handler = nexus_slab::try_box_dyn!(
///     msg_alloc::Allocator, dyn Handler<E>, my_handler
/// )?;
/// ```
#[macro_export]
macro_rules! try_box_dyn {
    ($alloc:ty, $dyn_ty:ty, $value:expr) => {{ <$crate::byte::BoxSlot<_, $alloc>>::try_new_as($value, |__p| __p as *mut $dyn_ty) }};
}

// =============================================================================
// Raw slab helpers — bounded::Slab<AlignedBytes<N>>
// =============================================================================

impl<const N: usize> crate::bounded::Slab<AlignedBytes<N>> {
    /// Inserts a value into the slab, returning a [`Slot`](byte::Slot) handle.
    ///
    /// Returns `Err(value)` if the slab is full.
    ///
    /// # Compile-Time Checks
    ///
    /// Fails to compile if `T` is too large or too aligned for the slot.
    #[inline]
    pub fn try_insert<T>(&self, value: T) -> Result<Slot<T>, T> {
        const {
            assert!(size_of::<T>() <= N, "T does not fit in byte slab slot");
        };
        const {
            assert!(
                align_of::<T>() <= align_of::<AlignedBytes<N>>(),
                "T alignment exceeds slot alignment"
            );
        };

        match self.claim_ptr() {
            Some(slot_ptr) => {
                // SAFETY: slot_ptr is valid and exclusively ours from claim_ptr.
                // T fits (const-asserted). SlotCell repr(C) union has data at
                // offset 0.
                unsafe {
                    let t_ptr = slot_ptr as *mut T;
                    ptr::write(t_ptr, value);
                    let t_size = size_of::<T>();
                    if t_size < N {
                        ptr::write_bytes((slot_ptr as *mut u8).add(t_size), 0, N - t_size);
                    }
                }
                // SAFETY: We just wrote a valid T at slot_ptr.
                Ok(unsafe { Slot::from_raw(slot_ptr as *mut T) })
            }
            None => Err(value),
        }
    }

    /// Drops the value and frees the slot.
    ///
    /// Handles both thin and fat pointers: extracts the data pointer for
    /// freeing regardless of whether `T` is `Sized` or `dyn Trait`.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this slab
    #[inline]
    pub unsafe fn remove<T: ?Sized>(&self, slot: Slot<T>) {
        let ptr = slot.into_raw();
        debug_assert!(
            self.contains_ptr(ptr as *const ()),
            "slot was not allocated from this slab"
        );
        // SAFETY: Caller guarantees slot came from this slab.
        unsafe {
            ptr::drop_in_place(ptr);
            let data_ptr = ptr as *mut () as *mut SlotCell<AlignedBytes<N>>;
            self.free_ptr(data_ptr);
        }
    }

    /// Extracts the value and frees the slot (Sized only).
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this slab
    #[inline]
    pub unsafe fn take_value<T>(&self, slot: Slot<T>) -> T {
        let ptr = slot.into_raw();
        debug_assert!(
            self.contains_ptr(ptr as *const ()),
            "slot was not allocated from this slab"
        );
        // SAFETY: Caller guarantees slot came from this slab.
        let value = unsafe { ptr::read(ptr) };
        let data_ptr = ptr as *mut () as *mut SlotCell<AlignedBytes<N>>;
        unsafe { self.free_ptr(data_ptr) };
        value
    }

    /// Frees the slot without dropping the value.
    ///
    /// Use when the value has already been moved out or dropped.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this slab
    /// - The value must already be dropped or moved out
    #[inline]
    pub unsafe fn reclaim<T: ?Sized>(&self, slot: Slot<T>) {
        let ptr = slot.into_raw();
        debug_assert!(
            self.contains_ptr(ptr as *const ()),
            "slot was not allocated from this slab"
        );
        // SAFETY: Caller guarantees slot came from this slab and value handled.
        unsafe {
            let data_ptr = ptr as *mut () as *mut SlotCell<AlignedBytes<N>>;
            self.free_ptr(data_ptr);
        }
    }
}

// =============================================================================
// Raw slab helpers — unbounded::Slab<AlignedBytes<N>>
// =============================================================================

impl<const N: usize> crate::unbounded::Slab<AlignedBytes<N>> {
    /// Inserts a value into the slab, returning a [`Slot`](byte::Slot) handle.
    ///
    /// Always succeeds — grows the slab if needed.
    ///
    /// # Compile-Time Checks
    ///
    /// Fails to compile if `T` is too large or too aligned for the slot.
    #[inline]
    pub fn insert<T>(&self, value: T) -> Slot<T> {
        const {
            assert!(size_of::<T>() <= N, "T does not fit in byte slab slot");
        };
        const {
            assert!(
                align_of::<T>() <= align_of::<AlignedBytes<N>>(),
                "T alignment exceeds slot alignment"
            );
        };

        let (slot_ptr, _chunk_idx) = self.claim_ptr();
        // SAFETY: slot_ptr is valid and exclusively ours from claim_ptr.
        unsafe {
            let t_ptr = slot_ptr as *mut T;
            ptr::write(t_ptr, value);
            let t_size = size_of::<T>();
            if t_size < N {
                ptr::write_bytes((slot_ptr as *mut u8).add(t_size), 0, N - t_size);
            }
        }
        // SAFETY: We just wrote a valid T at slot_ptr.
        unsafe { Slot::from_raw(slot_ptr as *mut T) }
    }

    /// Drops the value and frees the slot.
    ///
    /// Handles both thin and fat pointers: extracts the data pointer for
    /// freeing regardless of whether `T` is `Sized` or `dyn Trait`.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this slab
    #[inline]
    pub unsafe fn remove<T: ?Sized>(&self, slot: Slot<T>) {
        let ptr = slot.into_raw();
        debug_assert!(
            self.contains_ptr(ptr as *const ()),
            "slot was not allocated from this slab"
        );
        // SAFETY: Caller guarantees slot came from this slab.
        unsafe {
            ptr::drop_in_place(ptr);
            let data_ptr = ptr as *mut () as *mut SlotCell<AlignedBytes<N>>;
            self.free_ptr(data_ptr);
        }
    }

    /// Extracts the value and frees the slot (Sized only).
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this slab
    #[inline]
    pub unsafe fn take_value<T>(&self, slot: Slot<T>) -> T {
        let ptr = slot.into_raw();
        debug_assert!(
            self.contains_ptr(ptr as *const ()),
            "slot was not allocated from this slab"
        );
        // SAFETY: Caller guarantees slot came from this slab.
        let value = unsafe { ptr::read(ptr) };
        let data_ptr = ptr as *mut () as *mut SlotCell<AlignedBytes<N>>;
        unsafe { self.free_ptr(data_ptr) };
        value
    }

    /// Frees the slot without dropping the value.
    ///
    /// Use when the value has already been moved out or dropped.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this slab
    /// - The value must already be dropped or moved out
    #[inline]
    pub unsafe fn reclaim<T: ?Sized>(&self, slot: Slot<T>) {
        let ptr = slot.into_raw();
        debug_assert!(
            self.contains_ptr(ptr as *const ()),
            "slot was not allocated from this slab"
        );
        // SAFETY: Caller guarantees slot came from this slab and value handled.
        unsafe {
            let data_ptr = ptr as *mut () as *mut SlotCell<AlignedBytes<N>>;
            self.free_ptr(data_ptr);
        }
    }
}
