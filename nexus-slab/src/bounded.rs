//! Fixed-capacity slab allocator with RAII Entry-based access.
//!
//! [`BoundedSlab`] provides a pre-allocated, leaked slab where all memory is
//! allocated upfront and lives for `'static`. Operations are O(1) with no
//! allocations after initialization.
//!
//! # Design Philosophy
//!
//! **This is an allocator, not a data structure.**
//!
//! Like `malloc`/`free`:
//! - The allocator lives forever (leaked on creation)
//! - Handles are lightweight views (`Copy`, `!Send`)
//! - Entries own their slots (RAII - drop deallocates)
//! - Slot lifecycle is explicit (insert creates, drop/leak frees)
//!
//! # Example
//!
//! ```
//! use nexus_slab::BoundedSlab;
//!
//! let slab = BoundedSlab::leak(1024);
//!
//! // RAII entry - slot freed when entry drops
//! {
//!     let entry = slab.try_insert("hello").unwrap();
//!     assert_eq!(*entry.get(), "hello");
//! } // entry drops, slot freed
//!
//! // Leak to keep data alive
//! let entry = slab.try_insert("world").unwrap();
//! let key = entry.leak(); // data stays, returns Key
//!
//! // Access via key
//! assert_eq!(*slab.get(key).unwrap(), "world");
//! ```
//!
//! # Self-Referential Patterns
//!
//! ```
//! use nexus_slab::{BoundedSlab, Key};
//!
//! struct Node {
//!     self_key: Key,
//!     data: u64,
//! }
//!
//! let slab = BoundedSlab::leak(16);
//!
//! let entry = slab.try_insert_with(|e| Node {
//!     self_key: e.key(),
//!     data: 42,
//! }).unwrap();
//!
//! assert_eq!(entry.get().self_key, entry.key());
//! ```

use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::{Index, IndexMut};

use crate::shared::{SlotCell, SLOT_NONE};
use crate::{CapacityError, Entry, FreeFn, FreeSlotVTable, Full, Key, Ref, RefMut};

// =============================================================================
// BoundedSlabInner
// =============================================================================

/// Internal state for a fixed-capacity slab.
pub(crate) struct BoundedSlabInner<T> {
    pub(crate) slots: ManuallyDrop<Vec<SlotCell<T>>>,
    pub(crate) capacity: u32,
    pub(crate) free_head: Cell<u32>,
    pub(crate) len: Cell<u32>,
}

impl<T> BoundedSlabInner<T> {
    pub(crate) fn with_capacity(capacity: u32) -> Self {
        assert!(capacity > 0, "capacity must be non-zero");
        assert!(capacity <= SLOT_NONE, "capacity exceeds maximum");

        let mut slots: Vec<SlotCell<T>> = Vec::with_capacity(capacity as usize);

        for i in 0..capacity {
            let next = if i + 1 < capacity { i + 1 } else { SLOT_NONE };
            slots.push(SlotCell::new_vacant(next));
        }

        Self {
            slots: ManuallyDrop::new(slots),
            capacity,
            free_head: Cell::new(0),
            len: Cell::new(0),
        }
    }

    #[inline]
    pub(crate) fn slot(&self, index: u32) -> &SlotCell<T> {
        debug_assert!(index < self.capacity);
        unsafe { self.slots.get_unchecked(index as usize) }
    }

    #[inline]
    pub(crate) fn is_full(&self) -> bool {
        self.free_head.get() == SLOT_NONE
    }
}

// Note: No Drop impl - this is leaked and never dropped

// =============================================================================
// Free Function
// =============================================================================

/// Frees a slot back to the BoundedSlab's freelist.
///
/// # Safety
///
/// - `key` must be valid for this slab
/// - `ctx` must be a valid `*mut BoundedSlabInner<T>`
/// - Slot must be occupied (caller responsible for dropping value first)
pub(crate) unsafe fn bounded_slab_free<T>(key: Key, ctx: *mut ()) {
    let inner = ctx as *mut BoundedSlabInner<T>;
    let slot_index = key.index();

    // SAFETY: ctx is a valid BoundedSlabInner pointer, key is valid for this slab
    unsafe {
        let slot = (*inner).slot(slot_index);

        // Return slot to freelist (LIFO)
        let free_head = (*inner).free_head.get();
        slot.set_vacant(free_head);
        (*inner).free_head.set(slot_index);
        (*inner).len.set((*inner).len.get() - 1);
    }
}

// =============================================================================
// BoundedSlab
// =============================================================================

/// A fixed-capacity slab allocator with RAII Entry-based access.
///
/// Created via [`leak`](Self::leak), which allocates and leaks the slab,
/// returning a `Copy` handle. The slab lives for `'static`.
///
/// # Thread Safety
///
/// `BoundedSlab` is `!Send` and `!Sync` - it uses raw pointers internally.
/// The slab must only be used from the thread that created it.
///
/// # Example
///
/// ```
/// use nexus_slab::BoundedSlab;
///
/// let slab = BoundedSlab::leak(1024);
///
/// let entry = slab.try_insert(42).unwrap();
/// assert_eq!(*entry.get(), 42);
/// // entry drops, slot freed
/// ```
#[derive(Clone, Copy)]
pub struct BoundedSlab<T> {
    pub(crate) ptr: *mut BoundedSlabInner<T>,
    vtable: *const FreeSlotVTable,
    _marker: PhantomData<*mut ()>, // Ensures !Send + !Sync
}

impl<T> BoundedSlab<T> {
    #[inline]
    fn inner(&self) -> &BoundedSlabInner<T> {
        // SAFETY: ptr is valid for 'static (leaked)
        unsafe { &*self.ptr }
    }

    /// Creates and leaks a slab with the given capacity, returning a `Copy` handle.
    ///
    /// The slab lives for `'static` - it is never dropped. This is intentional:
    /// the slab is a dedicated allocator, not a temporary data structure.
    ///
    /// # Panics
    ///
    /// Panics if capacity is zero or exceeds maximum (~1 billion).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let slab = BoundedSlab::<String>::leak(1024);
    /// assert_eq!(slab.capacity(), 1024);
    /// ```
    pub fn leak(capacity: usize) -> Self {
        // 1. Allocate uninit box and leak immediately to get stable address
        let inner_uninit = Box::<BoundedSlabInner<T>>::new_uninit();
        let inner_ptr = Box::into_raw(inner_uninit) as *mut BoundedSlabInner<T>;

        // 2. Create and leak vtable (needs inner_ptr which is now valid forever)
        let vtable = Box::leak(Box::new(FreeSlotVTable {
            inner: inner_ptr as *mut (),
            free_fn: bounded_slab_free::<T> as FreeFn,
        }));

        // 3. Initialize inner in place through the leaked pointer
        let inner_data = BoundedSlabInner::with_capacity(capacity as u32);
        unsafe {
            inner_ptr.write(inner_data);
        }

        Self {
            ptr: inner_ptr,
            vtable,
            _marker: PhantomData,
        }
    }

    /// Returns the total capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner().capacity as usize
    }

    /// Returns the number of occupied slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner().len.get() as usize
    }

    /// Returns `true` if no slots are occupied.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner().len.get() == 0
    }

    /// Returns `true` if all slots are occupied.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.inner().is_full()
    }

    // =========================================================================
    // Insert
    // =========================================================================

    /// Inserts a value, returning an RAII Entry handle.
    ///
    /// The returned [`Entry`] owns the slot. When dropped, the slot is
    /// deallocated. Use [`Entry::leak()`] to keep the data alive.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full(value))` if the slab is full, allowing recovery
    /// of the rejected value.
    pub fn try_insert(&self, value: T) -> Result<Entry<T>, Full<T>> {
        let inner = self.inner();
        let free_head = inner.free_head.get();

        if free_head == SLOT_NONE {
            return Err(Full(value));
        }

        let slot = inner.slot(free_head);
        let next_free = slot.next_free();

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Ok(Entry::new(slot_ptr, self.vtable, Key::new(free_head)))
    }

    /// Inserts a value, panicking if full.
    ///
    /// # Panics
    ///
    /// Panics if the slab is full.
    pub fn insert(&self, value: T) -> Entry<T> {
        self.try_insert(value)
            .unwrap_or_else(|_| panic!("slab is full"))
    }

    /// Inserts with access to the entry before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if the slab is full. The closure is not
    /// called in this case.
    pub fn try_insert_with<F>(&self, f: F) -> Result<Entry<T>, CapacityError>
    where
        F: FnOnce(&Entry<T>) -> T,
    {
        let inner = self.inner();
        let free_head = inner.free_head.get();

        if free_head == SLOT_NONE {
            return Err(CapacityError);
        }

        let slot = inner.slot(free_head);
        let next_free = slot.next_free();

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();

        // Create entry (slot not yet occupied)
        let entry = Entry::new(slot_ptr, self.vtable, Key::new(free_head));

        // Call closure to get value
        let value = f(&entry);

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        Ok(entry)
    }

    /// Inserts with access to the entry, panicking if full.
    ///
    /// # Panics
    ///
    /// Panics if the slab is full.
    pub fn insert_with<F>(&self, f: F) -> Entry<T>
    where
        F: FnOnce(&Entry<T>) -> T,
    {
        self.try_insert_with(f).expect("slab is full")
    }

    /// Reserves a slot without filling it, returning a [`VacantEntry`].
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if the slab is full.
    pub fn try_vacant_entry(&self) -> Result<VacantEntry<T>, CapacityError> {
        let inner = self.inner();
        let free_head = inner.free_head.get();

        if free_head == SLOT_NONE {
            return Err(CapacityError);
        }

        let slot = inner.slot(free_head);
        let next_free = slot.next_free();

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        Ok(VacantEntry {
            ptr: self.ptr,
            vtable: self.vtable,
            key: Key::new(free_head),
            consumed: false,
            _marker: PhantomData,
        })
    }

    /// Reserves a slot without filling it, panicking if full.
    ///
    /// # Panics
    ///
    /// Panics if the slab is full.
    pub fn vacant_entry(&self) -> VacantEntry<T> {
        self.try_vacant_entry().expect("slab is full")
    }

    // =========================================================================
    // Key-based access
    // =========================================================================

    /// Returns `true` if the key refers to an occupied slot.
    #[inline]
    pub fn contains_key(&self, key: Key) -> bool {
        let index = key.index();
        let inner = self.inner();
        if index >= inner.capacity {
            return false;
        }
        inner.slot(index).is_occupied()
    }

    /// Alias for [`contains_key`](Self::contains_key).
    #[inline]
    pub fn contains(&self, key: Key) -> bool {
        self.contains_key(key)
    }

    /// Returns a tracked reference to the value at `key`.
    ///
    /// The returned [`Ref`] guard participates in runtime borrow tracking.
    #[inline]
    pub fn get(&self, key: Key) -> Option<Ref<T>> {
        let index = key.index();
        let inner = self.inner();
        if index >= inner.capacity {
            return None;
        }
        let slot = inner.slot(index);
        if !slot.is_available() {
            return None;
        }
        slot.set_borrowed();
        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Some(Ref::new(slot_ptr))
    }

    /// Returns a tracked mutable reference to the value at `key`.
    ///
    /// The returned [`RefMut`] guard participates in runtime borrow tracking.
    #[inline]
    pub fn get_mut(&self, key: Key) -> Option<RefMut<T>> {
        let index = key.index();
        let inner = self.inner();
        if index >= inner.capacity {
            return None;
        }
        let slot = inner.slot(index);
        if !slot.is_available() {
            return None;
        }
        slot.set_borrowed();
        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Some(RefMut::new(slot_ptr))
    }

    /// Creates an RAII Entry from a key.
    ///
    /// Returns `None` if the key is out of bounds or the slot is vacant.
    ///
    /// **Warning**: The returned entry owns the slot. When dropped, the slot
    /// is deallocated. Only call this when you want to take ownership.
    pub fn entry(&self, key: Key) -> Option<Entry<T>> {
        let index = key.index();
        let inner = self.inner();
        if index >= inner.capacity {
            return None;
        }
        let slot = inner.slot(index);
        if slot.is_vacant() {
            return None;
        }

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Some(Entry::new(slot_ptr, self.vtable, key))
    }

    /// Removes a value by key, bypassing RAII.
    ///
    /// Use this when you have a leaked key and want to deallocate.
    /// Returns `None` if the key is invalid or slot is vacant/borrowed.
    #[inline]
    pub fn remove_by_key(&self, key: Key) -> Option<T> {
        let index = key.index();
        let inner = self.inner();
        if index >= inner.capacity {
            return None;
        }

        let slot = inner.slot(index);
        if !slot.is_available() {
            return None;
        }

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let free_head = inner.free_head.get();
        slot.set_vacant(free_head);
        inner.free_head.set(index);
        inner.len.set(inner.len.get() - 1);

        Some(value)
    }

    /// Alias for [`remove_by_key`](Self::remove_by_key).
    #[inline]
    pub fn try_remove_by_key(&self, key: Key) -> Option<T> {
        self.remove_by_key(key)
    }

    /// Removes all values from the slab.
    pub fn clear(&self) {
        let inner = self.inner();

        if inner.len.get() == 0 {
            return;
        }

        for i in 0..inner.capacity {
            let slot = inner.slot(i);
            if slot.is_occupied() {
                unsafe {
                    std::ptr::drop_in_place((*slot.value.get()).as_mut_ptr());
                }
            }
            let next = if i + 1 < inner.capacity {
                i + 1
            } else {
                SLOT_NONE
            };
            slot.set_vacant(next);
        }

        inner.len.set(0);
        inner.free_head.set(0);
    }

    // =========================================================================
    // Unsafe access
    // =========================================================================

    /// Returns an untracked reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// No concurrent mutable access to this slot may exist.
    #[inline]
    pub unsafe fn get_untracked(&self, key: Key) -> Option<&T> {
        let index = key.index();
        let inner = self.inner();
        if index >= inner.capacity {
            return None;
        }
        let slot = inner.slot(index);
        if slot.is_vacant() {
            return None;
        }
        Some(unsafe { slot.value_ref() })
    }

    /// Returns an untracked mutable reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// No concurrent access to this slot may exist.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_untracked_mut(&self, key: Key) -> Option<&mut T> {
        let index = key.index();
        let inner = self.inner();
        if index >= inner.capacity {
            return None;
        }
        let slot = inner.slot(index);
        if slot.is_vacant() {
            return None;
        }
        Some(unsafe { slot.value_mut() })
    }

    /// Returns a reference without any checks.
    ///
    /// # Safety
    ///
    /// Key must be valid and slot must be occupied. No concurrent mutable access.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        let slot = self.inner().slot(key.index());
        unsafe { slot.value_ref() }
    }

    /// Returns a mutable reference without any checks.
    ///
    /// # Safety
    ///
    /// Key must be valid and slot must be occupied. No concurrent access.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_unchecked_mut(&self, key: Key) -> &mut T {
        let slot = self.inner().slot(key.index());
        unsafe { slot.value_mut() }
    }

    /// Gets an accessor for Index/IndexMut syntax.
    ///
    /// # Safety
    ///
    /// While this accessor is live, no Entry operations may occur.
    #[inline]
    pub unsafe fn untracked(&self) -> UntrackedAccessor<'_, T> {
        UntrackedAccessor(self)
    }

    // =========================================================================
    // Unchecked insert/remove
    // =========================================================================

    /// Inserts a value without checking capacity.
    ///
    /// # Safety
    ///
    /// Caller must ensure the slab is not full.
    #[inline]
    pub unsafe fn insert_unchecked(&self, value: T) -> Entry<T> {
        let inner = self.inner();
        let free_head = inner.free_head.get();

        debug_assert!(free_head != SLOT_NONE, "insert_unchecked on full slab");

        let slot = inner.slot(free_head);
        let next_free = slot.next_free();

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        unsafe { (*slot.value.get()).write(value) };
        slot.set_occupied();

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Entry::new(slot_ptr, self.vtable, Key::new(free_head))
    }

    /// Removes a value by key without bounds or occupancy checks.
    ///
    /// # Safety
    ///
    /// The key must be valid and the slot must be occupied.
    #[inline]
    pub unsafe fn remove_unchecked_by_key(&self, key: Key) -> T {
        let index = key.index();
        let inner = self.inner();
        let slot = inner.slot(index);

        debug_assert!(!slot.is_vacant(), "remove_unchecked_by_key on vacant slot");

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let free_head = inner.free_head.get();
        slot.set_vacant(free_head);
        inner.free_head.set(index);
        inner.len.set(inner.len.get() - 1);

        value
    }
}

impl<T> fmt::Debug for BoundedSlab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundedSlab")
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .finish()
    }
}

// =============================================================================
// UntrackedAccessor
// =============================================================================

/// Wrapper enabling Index/IndexMut syntax with untracked access.
///
/// This type bypasses runtime borrow tracking for performance.
///
/// # Safety
///
/// While this accessor is live, no Entry operations may occur on any slot.
pub struct UntrackedAccessor<'a, T>(&'a BoundedSlab<T>);

impl<T> Index<Key> for UntrackedAccessor<'_, T> {
    type Output = T;

    #[inline]
    fn index(&self, key: Key) -> &T {
        // SAFETY: Caller of untracked() guarantees no conflicting Entry ops
        unsafe { self.0.get_unchecked(key) }
    }
}

impl<T> IndexMut<Key> for UntrackedAccessor<'_, T> {
    #[inline]
    fn index_mut(&mut self, key: Key) -> &mut T {
        // SAFETY: Caller of untracked() guarantees no conflicting Entry ops
        unsafe { self.0.get_unchecked_mut(key) }
    }
}

impl<T> fmt::Debug for UntrackedAccessor<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UntrackedAccessor")
            .field("len", &self.0.len())
            .field("capacity", &self.0.capacity())
            .finish()
    }
}

// =============================================================================
// VacantEntry
// =============================================================================

/// A reserved but unfilled slot in the slab.
///
/// Created by [`BoundedSlab::try_vacant_entry`]. Fill with [`insert`](Self::insert)
/// or drop to return the slot to the freelist.
pub struct VacantEntry<T> {
    ptr: *mut BoundedSlabInner<T>,
    vtable: *const FreeSlotVTable,
    key: Key,
    consumed: bool,
    _marker: PhantomData<T>,
}

impl<T> VacantEntry<T> {
    #[inline]
    fn inner(&self) -> &BoundedSlabInner<T> {
        // SAFETY: ptr is valid for 'static
        unsafe { &*self.ptr }
    }

    /// Returns the key this slot will have once filled.
    #[inline]
    pub fn key(&self) -> Key {
        self.key
    }

    /// Fills the slot with a value, returning an RAII Entry.
    #[inline]
    pub fn insert(mut self, value: T) -> Entry<T> {
        // Scope the borrow of self to avoid conflict with consumed assignment
        let slot_ptr = {
            let slot = self.inner().slot(self.key.index());
            unsafe {
                (*slot.value.get()).write(value);
            }
            slot.set_occupied();
            (slot as *const SlotCell<T>).cast_mut()
        };

        self.consumed = true;

        Entry::new(slot_ptr, self.vtable, self.key)
    }
}

impl<T> Drop for VacantEntry<T> {
    fn drop(&mut self) {
        if !self.consumed {
            // Return slot to freelist
            let inner = self.inner();
            let slot = inner.slot(self.key.index());

            let free_head = inner.free_head.get();
            slot.set_vacant(free_head);
            inner.free_head.set(self.key.index());
            inner.len.set(inner.len.get() - 1);
        }
    }
}

impl<T> fmt::Debug for VacantEntry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VacantEntry")
            .field("key", &self.key)
            .finish()
    }
}

// =============================================================================
// Entry extension methods for bounded slab (take API)
// =============================================================================

impl<T> Entry<T> {
    /// Extracts the value, returning it with a [`VacantEntry`] for the slot.
    ///
    /// Unlike drop, this keeps the slot reserved.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or currently borrowed.
    pub fn take(self) -> (T, VacantEntry<T>) {
        self.try_take().expect("slot invalid or borrowed")
    }

    /// Extracts the value if valid, returning it with a [`VacantEntry`].
    pub fn try_take(self) -> Option<(T, VacantEntry<T>)> {
        let slot = self.slot();
        if !slot.is_available() {
            return None;
        }

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Get ptr from vtable
        let vtable = self.vtable();
        let ptr = unsafe { (*vtable).inner as *mut BoundedSlabInner<T> };

        let vacant = VacantEntry {
            ptr,
            vtable,
            key: self.key(),
            consumed: false,
            _marker: PhantomData,
        };

        // Don't run Entry's Drop (which would deallocate)
        std::mem::forget(self);

        Some((value, vacant))
    }

    /// Extracts the value without checks.
    ///
    /// # Safety
    ///
    /// Slot must be valid and not borrowed.
    pub unsafe fn take_unchecked(self) -> (T, VacantEntry<T>) {
        let slot = self.slot();

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Get ptr from vtable
        let vtable = self.vtable();
        let ptr = unsafe { (*vtable).inner as *mut BoundedSlabInner<T> };

        let vacant = VacantEntry {
            ptr,
            vtable,
            key: self.key(),
            consumed: false,
            _marker: PhantomData,
        };

        std::mem::forget(self);

        (value, vacant)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leak_basic() {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(100);
        assert_eq!(slab.capacity(), 100);
        assert_eq!(slab.len(), 0);
        assert!(slab.is_empty());
        assert!(!slab.is_full());
    }

    #[test]
    #[should_panic(expected = "capacity must be non-zero")]
    fn leak_zero_panics() {
        let _: BoundedSlab<u64> = BoundedSlab::leak(0);
    }

    #[test]
    fn insert_and_drop() {
        let slab = BoundedSlab::leak(16);

        {
            let entry = slab.try_insert(42u64).unwrap();
            assert_eq!(slab.len(), 1);
            assert_eq!(*entry.get(), 42);
        }

        // Entry dropped, slot freed
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn leak_keeps_data() {
        let slab = BoundedSlab::leak(16);

        let entry = slab.try_insert(100u64).unwrap();
        let key = entry.leak();

        // Data still exists
        assert_eq!(slab.len(), 1);
        assert_eq!(*slab.get(key).unwrap(), 100);

        // Clean up via remove
        let value = slab.remove_by_key(key).unwrap();
        assert_eq!(value, 100);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn entry_from_key() {
        let slab = BoundedSlab::leak(16);

        let entry = slab.try_insert(42u64).unwrap();
        let key = entry.leak();

        // Re-acquire RAII entry
        {
            let entry = slab.entry(key).unwrap();
            assert_eq!(*entry.get(), 42);
        }

        // Entry dropped, slot freed
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn insert_full_returns_error() {
        let slab = BoundedSlab::leak(2);

        let e1 = slab.try_insert(1u64).unwrap();
        let e2 = slab.try_insert(2u64).unwrap();

        let result = slab.try_insert(3u64);
        assert!(matches!(result, Err(Full(3))));

        // Clean up
        drop(e1);
        drop(e2);
    }

    #[test]
    fn vacant_entry_insert() {
        let slab = BoundedSlab::leak(16);

        let vacant = slab.try_vacant_entry().unwrap();
        let key = vacant.key();
        let entry = vacant.insert(format!("slot-{}", key.index()));

        assert_eq!(*entry.get(), format!("slot-{}", key.index()));
    }

    #[test]
    fn vacant_entry_drop() {
        let slab: BoundedSlab<u64> = BoundedSlab::leak(16);

        {
            let _vacant = slab.try_vacant_entry().unwrap();
            assert_eq!(slab.len(), 1);
        }

        // Vacant dropped without insert, slot returned
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn insert_with_self_reference() {
        let slab = BoundedSlab::leak(16);

        let entry = slab
            .try_insert_with(|e| (e.key(), 42u64))
            .unwrap();

        let (stored_key, value) = *entry.get();
        assert_eq!(stored_key, entry.key());
        assert_eq!(value, 42);
    }

    #[test]
    fn borrow_tracking() {
        let slab = BoundedSlab::leak(16);

        let entry = slab.try_insert(42u64).unwrap();

        {
            let _ref1 = entry.get();
            // Second borrow should fail while first is held
            assert!(entry.try_get().is_none());
        }

        // After drop, borrow succeeds
        let _ref2 = entry.get();
    }

    #[test]
    fn handle_is_copy() {
        let slab = BoundedSlab::leak(16);
        let slab2 = slab; // Copy
        let slab3 = slab; // Copy again

        let _e1 = slab.try_insert(1u64).unwrap();
        let _e2 = slab2.try_insert(2u64).unwrap();
        let _e3 = slab3.try_insert(3u64).unwrap();

        assert_eq!(slab.len(), 3);
    }

    #[test]
    fn entry_size() {
        // Entry is 24 bytes: slot ptr (8) + vtable ptr (8) + key (4) + padding (4)
        assert_eq!(std::mem::size_of::<Entry<u64>>(), 24);
    }

    #[test]
    fn take_and_reinsert() {
        let slab = BoundedSlab::leak(16);

        let entry = slab.try_insert(42u64).unwrap();
        let key = entry.key();

        let (value, vacant) = entry.take();
        assert_eq!(value, 42);
        assert_eq!(vacant.key(), key);

        let new_entry = vacant.insert(100);
        assert_eq!(new_entry.key(), key);
        assert_eq!(*new_entry.get(), 100);
    }

    #[test]
    fn replace() {
        let slab = BoundedSlab::leak(16);
        let entry = slab.try_insert(42u64).unwrap();

        let old = entry.replace(100);
        assert_eq!(old, 42);
        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn and_modify() {
        let slab = BoundedSlab::leak(16);
        let entry = slab.try_insert(0u64).unwrap();

        entry
            .and_modify(|v| *v += 1)
            .and_modify(|v| *v *= 2);

        assert_eq!(*entry.get(), 2);
    }

    #[test]
    fn explicit_remove() {
        let slab = BoundedSlab::leak(16);
        let entry = slab.try_insert(42u64).unwrap();

        let value = entry.remove();
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn clear() {
        let slab = BoundedSlab::leak(16);

        // Insert and leak some entries
        for i in 0..5 {
            let entry = slab.try_insert(i as u64).unwrap();
            entry.leak();
        }

        assert_eq!(slab.len(), 5);

        slab.clear();

        assert_eq!(slab.len(), 0);
        assert!(slab.is_empty());
    }
}
