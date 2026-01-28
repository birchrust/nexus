//! Fixed-capacity slab allocator with Entry-based access.
//!
//! [`BoundedSlab`] provides a pre-allocated slab where all memory is allocated
//! upfront. Operations are O(1) with no allocations after initialization.
//!
//! # Example
//!
//! ```
//! use nexus_slab::BoundedSlab;
//!
//! let slab = BoundedSlab::with_capacity(1024);
//!
//! let entry = slab.insert("hello").unwrap();
//! assert_eq!(*entry.get(), "hello");
//!
//! let value = entry.remove();
//! assert_eq!(value, "hello");
//! ```
//!
//! # Self-Referential Patterns
//!
//! ```
//! use nexus_slab::{BoundedSlab, Entry};
//!
//! struct Node {
//!     self_ref: Entry<Node>,
//!     data: u64,
//! }
//!
//! let slab = BoundedSlab::with_capacity(16);
//!
//! let entry = slab.insert_with(|e| Node {
//!     self_ref: e.clone(),
//!     data: 42,
//! }).unwrap();
//!
//! assert_eq!(entry.get().data, 42);
//! ```

use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut, Index, IndexMut};
use std::pin::Pin;
use std::rc::{Rc, Weak};

use crate::shared::{SlotCell, SLOT_NONE};
use crate::{CapacityError, Full, Key};

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

impl<T> Drop for BoundedSlabInner<T> {
    fn drop(&mut self) {
        for i in 0..self.capacity {
            let slot = self.slot(i);
            if slot.is_occupied() {
                unsafe {
                    std::ptr::drop_in_place((*slot.value.get()).as_mut_ptr());
                }
            }
        }

        unsafe {
            ManuallyDrop::drop(&mut self.slots);
        }
    }
}

// =============================================================================
// BoundedSlab
// =============================================================================

/// A fixed-capacity slab allocator with Entry-based access.
///
/// All memory is allocated upfront. Operations are O(1).
///
/// # Thread Safety
///
/// `BoundedSlab` is `!Send` and `!Sync` - it uses `Rc` internally for efficient
/// single-threaded operation.
///
/// # Example
///
/// ```
/// use nexus_slab::BoundedSlab;
///
/// let slab = BoundedSlab::with_capacity(1024);
///
/// let entry = slab.insert(42).unwrap();
/// assert_eq!(*entry.get(), 42);
///
/// let value = entry.remove();
/// assert_eq!(value, 42);
/// ```
pub struct BoundedSlab<T> {
    pub(crate) inner: Rc<BoundedSlabInner<T>>,
}

impl<T> BoundedSlab<T> {
    /// Creates a new slab with the given capacity.
    ///
    /// All slots are pre-allocated and initialized.
    ///
    /// # Panics
    ///
    /// Panics if capacity is zero or exceeds maximum (~1 billion).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Rc::new(BoundedSlabInner::with_capacity(capacity as u32)),
        }
    }

    /// Returns the total capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity as usize
    }

    /// Returns the number of occupied slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len.get() as usize
    }

    /// Returns `true` if no slots are occupied.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.len.get() == 0
    }

    /// Returns `true` if all slots are occupied.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    /// Inserts a value, returning an Entry handle.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full(value))` if the slab is full, allowing the caller
    /// to recover the rejected value.
    pub fn insert(&self, value: T) -> Result<Entry<T>, Full<T>> {
        let inner = &*self.inner;
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

        Ok(Entry {
            slab: Rc::downgrade(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
            index: free_head,
        })
    }

    /// Inserts with access to the Entry before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own Entry.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if the slab is full. The closure is not
    /// called in this case.
    pub fn insert_with<F>(&self, f: F) -> Result<Entry<T>, CapacityError>
    where
        F: FnOnce(Entry<T>) -> T,
    {
        let inner = &*self.inner;
        let free_head = inner.free_head.get();

        if free_head == SLOT_NONE {
            return Err(CapacityError);
        }

        let slot = inner.slot(free_head);
        let next_free = slot.next_free();

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        let entry = Entry {
            slab: Rc::downgrade(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
            index: free_head,
        };

        let value = f(entry.clone());

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        Ok(entry)
    }

    /// Creates an Entry from a key.
    ///
    /// Returns `None` if the key is out of bounds or the slot is vacant.
    pub fn entry(&self, key: Key) -> Option<Entry<T>> {
        let index = key.index();
        if index >= self.inner.capacity {
            return None;
        }

        let slot = self.inner.slot(index);
        if slot.is_vacant() {
            return None;
        }

        Some(Entry {
            slab: Rc::downgrade(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
            index,
        })
    }

    /// Reserves a slot without filling it, returning a [`VacantEntry`].
    ///
    /// The `VacantEntry` can be used to get the key before constructing
    /// the value, then fill the slot via [`VacantEntry::insert`].
    ///
    /// If the `VacantEntry` is dropped without calling `insert`, the slot
    /// is automatically returned to the freelist.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if the slab is full.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let slab = BoundedSlab::with_capacity(16);
    ///
    /// let vacant = slab.vacant_entry().unwrap();
    /// let key = vacant.key();
    ///
    /// // Now we know the key before creating the value
    /// let entry = vacant.insert(format!("item-{}", key.index()));
    /// assert!(entry.get().starts_with("item-"));
    /// ```
    pub fn vacant_entry(&self) -> Result<VacantEntry<T>, CapacityError> {
        let inner = &*self.inner;
        let free_head = inner.free_head.get();

        if free_head == SLOT_NONE {
            return Err(CapacityError);
        }

        let slot = inner.slot(free_head);
        let next_free = slot.next_free();

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        // Note: slot is NOT marked occupied yet - that happens in VacantEntry::insert

        Ok(VacantEntry {
            slab: Rc::downgrade(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
            index: free_head,
            _marker: PhantomData,
        })
    }

    /// Removes a value via its Entry handle.
    ///
    /// This is faster than [`Entry::remove`] because it skips the
    /// `Weak::upgrade()` liveness check - the slab already has the `Rc`.
    ///
    /// # Panics
    ///
    /// Panics if the slot is vacant or borrowed.
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub fn remove(&self, entry: Entry<T>) -> T {
        self.try_remove(entry).expect("slot is vacant or borrowed")
    }

    /// Removes a value via its Entry handle, returning `None` if invalid.
    ///
    /// This is the non-panicking version of [`remove`](Self::remove).
    /// Returns `None` if the slot is vacant or currently borrowed.
    ///
    /// This is faster than [`Entry::try_remove`] because it skips the
    /// `Weak::upgrade()` liveness check - the slab already has the `Rc`.
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub fn try_remove(&self, entry: Entry<T>) -> Option<T> {
        let slot = unsafe { &*entry.slot_ptr };

        if !slot.is_available() {
            return None;
        }

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let free_head = self.inner.free_head.get();
        slot.set_vacant(free_head);
        self.inner.free_head.set(entry.index);
        self.inner.len.set(self.inner.len.get() - 1);

        Some(value)
    }

    /// Removes all values from the slab.
    pub fn clear(&self) {
        let inner = &*self.inner;

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
    // Key-based access (for collections compatibility)
    // =========================================================================

    /// Returns `true` if the key refers to an occupied slot.
    #[inline]
    pub fn contains_key(&self, key: Key) -> bool {
        let index = key.index();
        if index >= self.inner.capacity {
            return false;
        }
        self.inner.slot(index).is_occupied()
    }

    /// Alias for [`contains_key`](Self::contains_key) for API compatibility.
    #[inline]
    pub fn contains(&self, key: Key) -> bool {
        self.contains_key(key)
    }

    /// Returns a tracked reference to the value at `key`.
    ///
    /// The returned [`Ref`] guard participates in runtime borrow tracking,
    /// preventing conflicting access while the guard is held.
    #[inline]
    pub fn get(&self, key: Key) -> Option<Ref<T>> {
        let index = key.index();
        if index >= self.inner.capacity {
            return None;
        }
        let slot = self.inner.slot(index);
        if !slot.is_available() {
            return None;
        }
        slot.set_borrowed();
        Some(Ref {
            _slab: Rc::clone(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
        })
    }

    /// Returns a tracked mutable reference to the value at `key`.
    ///
    /// The returned [`RefMut`] guard participates in runtime borrow tracking,
    /// preventing conflicting access while the guard is held.
    #[inline]
    pub fn get_mut(&self, key: Key) -> Option<RefMut<T>> {
        let index = key.index();
        if index >= self.inner.capacity {
            return None;
        }
        let slot = self.inner.slot(index);
        if !slot.is_available() {
            return None;
        }
        slot.set_borrowed();
        Some(RefMut {
            _slab: Rc::clone(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
        })
    }

    // =========================================================================
    // Untracked access (unsafe - bypasses borrow tracking)
    // =========================================================================

    /// Returns an untracked reference to the value at `key`.
    ///
    /// This bypasses runtime borrow tracking for performance. The validity
    /// of the key is still checked.
    ///
    /// # Safety
    ///
    /// Caller must ensure no conflicting Entry operations (remove, replace,
    /// get_mut) occur on this slot while the reference is live. Mixing
    /// untracked access with Entry operations on the same slot is unsound.
    #[inline]
    pub unsafe fn get_untracked(&self, key: Key) -> Option<&T> {
        let index = key.index();
        if index >= self.inner.capacity {
            return None;
        }
        let slot = self.inner.slot(index);
        if slot.is_vacant() {
            return None;
        }
        Some(unsafe { slot.value_ref() })
    }

    /// Returns an untracked mutable reference to the value at `key`.
    ///
    /// This bypasses runtime borrow tracking for performance. The validity
    /// of the key is still checked.
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access and no conflicting Entry operations
    /// (remove, replace, get, get_mut) occur on this slot while the reference
    /// is live. Mixing untracked access with Entry operations is unsound.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_untracked_mut(&self, key: Key) -> Option<&mut T> {
        let index = key.index();
        if index >= self.inner.capacity {
            return None;
        }
        let slot = self.inner.slot(index);
        if slot.is_vacant() {
            return None;
        }
        Some(unsafe { slot.value_mut() })
    }

    /// Returns an untracked reference without any checks.
    ///
    /// # Safety
    ///
    /// - The key must be valid and the slot must be occupied.
    /// - No conflicting Entry operations while reference is live.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        unsafe { self.inner.slot(key.index()).value_ref() }
    }

    /// Returns an untracked mutable reference without any checks.
    ///
    /// # Safety
    ///
    /// - The key must be valid and the slot must be occupied.
    /// - Caller must ensure exclusive access.
    /// - No conflicting Entry operations while reference is live.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_unchecked_mut(&self, key: Key) -> &mut T {
        unsafe { self.inner.slot(key.index()).value_mut() }
    }

    /// Returns an [`UntrackedAccessor`] for Index/IndexMut syntax.
    ///
    /// # Safety
    ///
    /// While the accessor or any reference from it is live, caller must not
    /// perform Entry operations that could invalidate references (remove,
    /// take, replace, get_mut on same slot).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let slab = BoundedSlab::with_capacity(16);
    /// let entry = slab.insert(42u64).unwrap();
    /// let key = entry.key();
    ///
    /// // SAFETY: No Entry operations in this scope
    /// let accessor = unsafe { slab.untracked() };
    /// assert_eq!(accessor[key], 42);
    /// ```
    #[inline]
    pub unsafe fn untracked(&self) -> UntrackedAccessor<'_, T> {
        UntrackedAccessor(self)
    }

    /// Removes and returns the value at `key`.
    ///
    /// # Panics
    ///
    /// Panics if the key is invalid, the slot is vacant, or borrowed.
    #[inline]
    pub fn remove_by_key(&self, key: Key) -> T {
        self.try_remove_by_key(key)
            .expect("key invalid, slot vacant, or borrowed")
    }

    /// Removes a value by key, returning `None` if invalid.
    ///
    /// This is the non-panicking version of [`remove_by_key`](Self::remove_by_key).
    /// Returns `None` if the key is out of bounds, the slot is vacant, or borrowed.
    #[inline]
    pub fn try_remove_by_key(&self, key: Key) -> Option<T> {
        let index = key.index();
        if index >= self.inner.capacity {
            return None;
        }

        let slot = self.inner.slot(index);
        if !slot.is_available() {
            return None;
        }

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let free_head = self.inner.free_head.get();
        slot.set_vacant(free_head);
        self.inner.free_head.set(index);
        self.inner.len.set(self.inner.len.get() - 1);

        Some(value)
    }

    // =========================================================================
    // Unchecked API
    // =========================================================================

    /// Inserts a value without checking capacity.
    ///
    /// # Safety
    ///
    /// Caller must ensure the slab is not full.
    #[inline]
    pub unsafe fn insert_unchecked(&self, value: T) -> Entry<T> {
        let inner = &*self.inner;
        let free_head = inner.free_head.get();

        debug_assert!(free_head != SLOT_NONE, "insert_unchecked on full slab");

        let slot = inner.slot(free_head);
        let next_free = slot.next_free();

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        unsafe { (*slot.value.get()).write(value) };
        slot.set_occupied();

        Entry {
            slab: Rc::downgrade(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
            index: free_head,
        }
    }

    /// Removes a value by key without bounds or occupancy checks.
    ///
    /// # Safety
    ///
    /// The key must be valid and the slot must be occupied.
    #[inline]
    pub unsafe fn remove_unchecked_by_key(&self, key: Key) -> T {
        let index = key.index();
        let slot = self.inner.slot(index);

        debug_assert!(!slot.is_vacant(), "remove_unchecked_by_key on vacant slot");

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let free_head = self.inner.free_head.get();
        slot.set_vacant(free_head);
        self.inner.free_head.set(index);
        self.inner.len.set(self.inner.len.get() - 1);

        value
    }
}

impl<T> Clone for BoundedSlab<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
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
/// This type bypasses runtime borrow tracking for performance. It is obtained
/// via [`BoundedSlab::untracked()`] which is unsafe.
///
/// # Safety
///
/// While this accessor (or any reference obtained from it) is live, the caller
/// must not perform Entry operations that could invalidate references:
/// - `Entry::remove()`
/// - `Entry::take()`
/// - `Entry::replace()`
/// - `Entry::get_mut()` on the same slot
///
/// Violating this leads to undefined behavior (dangling references).
///
/// # Example
///
/// ```
/// use nexus_slab::BoundedSlab;
///
/// let slab = BoundedSlab::with_capacity(16);
/// let entry = slab.insert(42u64).unwrap();
/// let key = entry.key();
///
/// // SAFETY: No Entry operations while accessor is in use
/// let accessor = unsafe { slab.untracked() };
/// assert_eq!(accessor[key], 42);
/// ```
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
/// Created by [`BoundedSlab::vacant_entry`], this represents a slot that has
/// been claimed from the freelist but not yet filled with a value.
///
/// Use [`insert`](Self::insert) to fill the slot and get an [`Entry`] handle.
/// If dropped without calling `insert`, the slot is automatically returned
/// to the freelist.
///
/// # Use Cases
///
/// - Get the key before constructing the value (without a closure)
/// - Two-phase initialization patterns
/// - More explicit control flow than [`insert_with`](BoundedSlab::insert_with)
///
/// # Example
///
/// ```
/// use nexus_slab::BoundedSlab;
///
/// let slab = BoundedSlab::with_capacity(16);
///
/// // Reserve a slot and get its key
/// let vacant = slab.vacant_entry().unwrap();
/// let key = vacant.key();
///
/// // Use the key to construct the value
/// let entry = vacant.insert(format!("slot-{}", key.index()));
/// assert_eq!(*entry.get(), format!("slot-{}", key.index()));
/// ```
pub struct VacantEntry<T> {
    slab: Weak<BoundedSlabInner<T>>,
    slot_ptr: *const SlotCell<T>,
    index: u32,
    _marker: PhantomData<T>,
}

impl<T> VacantEntry<T> {
    /// Returns the key this slot will have once filled.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.index)
    }

    /// Fills the slot with a value, returning an [`Entry`] handle.
    ///
    /// Consumes the `VacantEntry`, preventing the slot from being
    /// returned to the freelist.
    ///
    /// # Panics
    ///
    /// Panics if the slab has been dropped while holding this `VacantEntry`.
    #[inline]
    pub fn insert(self, value: T) -> Entry<T> {
        // Verify slab is still alive
        let _inner = self
            .slab
            .upgrade()
            .expect("slab dropped while holding VacantEntry");

        let slot = unsafe { &*self.slot_ptr };

        // Write the value and mark occupied
        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        // Create Entry before forgetting self
        let entry = Entry {
            slab: self.slab.clone(),
            slot_ptr: self.slot_ptr,
            index: self.index,
        };

        // Prevent Drop from returning slot to freelist
        std::mem::forget(self);

        entry
    }
}

impl<T> Drop for VacantEntry<T> {
    fn drop(&mut self) {
        // Return slot to freelist if slab still exists
        if let Some(inner) = self.slab.upgrade() {
            let slot = unsafe { &*self.slot_ptr };

            let free_head = inner.free_head.get();
            slot.set_vacant(free_head);
            inner.free_head.set(self.index);
            inner.len.set(inner.len.get() - 1);
        }
    }
}

impl<T> fmt::Debug for VacantEntry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VacantEntry")
            .field("index", &self.index)
            .field("alive", &(self.slab.strong_count() > 0))
            .finish()
    }
}

// =============================================================================
// Entry
// =============================================================================

/// A handle to a value in the slab.
///
/// Entry holds a direct pointer to the slot, enabling O(1) access without
/// bounds checking. It also holds a `Weak` reference for liveness checking.
///
/// # Size
///
/// Entry is 24 bytes: 8 (Weak) + 8 (pointer) + 4 (index) + 4 (padding).
pub struct Entry<T> {
    pub(crate) slab: Weak<BoundedSlabInner<T>>,
    pub(crate) slot_ptr: *const SlotCell<T>,
    pub(crate) index: u32,
}

impl<T> Entry<T> {
    /// Returns the key for use with collections.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.index)
    }

    /// Returns `true` if the slab still exists.
    #[inline]
    pub fn is_alive(&self) -> bool {
        self.slab.strong_count() > 0
    }

    /// Returns `true` if the entry is valid (slab alive, slot occupied).
    ///
    /// Does not check or set the borrow flag. Use this for quick validity
    /// checks without acquiring a borrow.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.slab.strong_count() > 0 && unsafe { &*self.slot_ptr }.is_occupied()
    }

    // =========================================================================
    // Safe API (panics on invalid state)
    // =========================================================================

    /// Returns a reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The slab has been dropped
    /// - The slot is vacant (entry was removed)
    /// - The slot is already borrowed
    pub fn get(&self) -> Ref<T> {
        self.try_get().expect("entry is invalid or borrowed")
    }

    /// Returns a mutable reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if the slab was dropped, slot is vacant, or already borrowed.
    pub fn get_mut(&self) -> RefMut<T> {
        self.try_get_mut().expect("entry is invalid or borrowed")
    }

    /// Returns a pinned reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if the slab was dropped, slot is vacant, or already borrowed.
    pub fn get_pinned(&self) -> Pin<Ref<T>> {
        unsafe { Pin::new_unchecked(self.get()) }
    }

    /// Returns a pinned mutable reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if the slab was dropped, slot is vacant, or already borrowed.
    pub fn get_pinned_mut(&self) -> Pin<RefMut<T>> {
        unsafe { Pin::new_unchecked(self.get_mut()) }
    }

    /// Removes and returns the value.
    ///
    /// # Panics
    ///
    /// Panics if the slab was dropped, slot is vacant, or currently borrowed.
    pub fn remove(self) -> T {
        self.try_remove().expect("entry is invalid or borrowed")
    }

    // =========================================================================
    // Try API (returns Option)
    // =========================================================================

    /// Returns a reference to the value, or `None` if invalid.
    ///
    /// Returns `None` if:
    /// - The slab has been dropped
    /// - The slot is vacant (entry was removed)
    /// - The slot is already borrowed
    pub fn try_get(&self) -> Option<Ref<T>> {
        let slab = self.slab.upgrade()?;
        let slot = unsafe { &*self.slot_ptr };

        if !slot.is_available() {
            return None;
        }

        slot.set_borrowed();

        Some(Ref {
            _slab: slab,
            slot_ptr: self.slot_ptr,
        })
    }

    /// Returns a mutable reference to the value, or `None` if invalid.
    pub fn try_get_mut(&self) -> Option<RefMut<T>> {
        let slab = self.slab.upgrade()?;
        let slot = unsafe { &*self.slot_ptr };

        if !slot.is_available() {
            return None;
        }

        slot.set_borrowed();

        Some(RefMut {
            _slab: slab,
            slot_ptr: self.slot_ptr,
        })
    }

    /// Returns a pinned reference to the value, or `None` if invalid.
    pub fn try_get_pinned(&self) -> Option<Pin<Ref<T>>> {
        self.try_get().map(|r| unsafe { Pin::new_unchecked(r) })
    }

    /// Returns a pinned mutable reference to the value, or `None` if invalid.
    pub fn try_get_pinned_mut(&self) -> Option<Pin<RefMut<T>>> {
        self.try_get_mut().map(|r| unsafe { Pin::new_unchecked(r) })
    }

    /// Removes and returns the value, or `None` if invalid.
    ///
    /// Returns `None` if the slab has been dropped, the slot is vacant,
    /// or the slot is currently borrowed.
    pub fn try_remove(self) -> Option<T> {
        let inner = self.slab.upgrade()?;
        let slot = unsafe { &*self.slot_ptr };

        if !slot.is_available() {
            return None;
        }

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let slots_ptr = inner.slots.as_ptr();
        let local_idx = ((self.slot_ptr as usize - slots_ptr as usize)
            / std::mem::size_of::<SlotCell<T>>()) as u32;

        let free_head = inner.free_head.get();
        slot.set_vacant(free_head);
        inner.free_head.set(local_idx);
        inner.len.set(inner.len.get() - 1);

        Some(value)
    }

    // =========================================================================
    // Untracked API (bypasses borrow tracking, still checks validity)
    // =========================================================================

    /// Returns an untracked reference if the entry is valid.
    ///
    /// This bypasses runtime borrow tracking for performance.
    ///
    /// # Safety
    ///
    /// Caller must ensure no conflicting operations occur on this slot
    /// while the reference is live:
    /// - No `remove()` on this entry or by key
    /// - No `get_mut()` or `get_untracked_mut()` on this entry
    /// - No `replace()` on this entry
    #[inline]
    pub unsafe fn get_untracked(&self) -> Option<&T> {
        if self.slab.strong_count() == 0 {
            return None;
        }
        let slot = unsafe { &*self.slot_ptr };
        if slot.is_vacant() {
            return None;
        }
        Some(unsafe { slot.value_ref() })
    }

    /// Returns an untracked mutable reference if the entry is valid.
    ///
    /// This bypasses runtime borrow tracking for performance.
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access and no conflicting operations
    /// occur on this slot while the reference is live:
    /// - No `remove()` on this entry or by key
    /// - No `get()`, `get_mut()`, or other access on this entry
    /// - No `replace()` on this entry
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_untracked_mut(&self) -> Option<&mut T> {
        if self.slab.strong_count() == 0 {
            return None;
        }
        let slot = unsafe { &*self.slot_ptr };
        if slot.is_vacant() {
            return None;
        }
        Some(unsafe { slot.value_mut() })
    }

    // =========================================================================
    // Unchecked API (no tracking, no validity checks)
    // =========================================================================

    /// Direct read without any checks.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - Slab is still alive
    /// - Slot is occupied
    /// - No concurrent mutable access
    /// - No conflicting Entry operations while reference is live
    #[inline(always)]
    pub unsafe fn get_unchecked(&self) -> &T {
        unsafe { (*self.slot_ptr).value_ref() }
    }

    /// Direct write without any checks.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - Slab is still alive
    /// - Slot is occupied
    /// - Exclusive access
    /// - No conflicting Entry operations while reference is live
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_unchecked_mut(&self) -> &mut T {
        unsafe { (*self.slot_ptr).value_mut() }
    }

    /// Pinned read without checks.
    ///
    /// # Safety
    ///
    /// Same requirements as [`get_unchecked`](Self::get_unchecked).
    #[inline(always)]
    pub unsafe fn get_pinned_unchecked(&self) -> Pin<&T> {
        unsafe { Pin::new_unchecked(self.get_unchecked()) }
    }

    /// Pinned write without checks.
    ///
    /// # Safety
    ///
    /// Same requirements as [`get_unchecked_mut`](Self::get_unchecked_mut).
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_pinned_mut_unchecked(&self) -> Pin<&mut T> {
        unsafe { Pin::new_unchecked(self.get_unchecked_mut()) }
    }

    /// Remove without any checks.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - Slab is still alive
    /// - Slot is occupied
    /// - No active borrows (tracked or untracked)
    pub unsafe fn remove_unchecked(self) -> T {
        let inner = unsafe { self.slab.upgrade().unwrap_unchecked() };
        let slot = unsafe { &*self.slot_ptr };

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let slots_ptr = inner.slots.as_ptr();
        let local_idx =
            (self.slot_ptr as usize - slots_ptr as usize) / std::mem::size_of::<SlotCell<T>>();

        let free_head = inner.free_head.get();
        slot.set_vacant(free_head);
        inner.free_head.set(local_idx as u32);
        inner.len.set(inner.len.get() - 1);

        value
    }

    // =========================================================================
    // Replace API
    // =========================================================================

    /// Replaces the value, returning the old one.
    ///
    /// The slot remains occupied with the new value.
    ///
    /// # Panics
    ///
    /// Panics if the slab was dropped, slot is vacant, or currently borrowed.
    pub fn replace(&self, value: T) -> T {
        self.try_replace(value).expect("entry is invalid or borrowed")
    }

    /// Replaces the value if valid, returning the old one.
    ///
    /// Returns `None` if the slab has been dropped, the slot is vacant,
    /// or the slot is currently borrowed. In this case, `value` is dropped.
    ///
    /// To recover the value on failure, use [`try_replace_with`](Self::try_replace_with).
    pub fn try_replace(&self, value: T) -> Option<T> {
        self.try_replace_with(|_| value)
    }

    /// Replaces the value using a closure, returning the old value.
    ///
    /// The closure receives a reference to the old value before replacement.
    ///
    /// # Panics
    ///
    /// Panics if the slab was dropped, slot is vacant, or currently borrowed.
    pub fn replace_with<F>(&self, f: F) -> T
    where
        F: FnOnce(&T) -> T,
    {
        self.try_replace_with(f)
            .expect("entry is invalid or borrowed")
    }

    /// Replaces the value using a closure if valid, returning the old value.
    ///
    /// Returns `None` if the entry is invalid or borrowed. The closure is
    /// not called in this case.
    pub fn try_replace_with<F>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&T) -> T,
    {
        let _inner = self.slab.upgrade()?;
        let slot = unsafe { &*self.slot_ptr };

        if !slot.is_available() {
            return None;
        }

        // Read old value
        let old_value = unsafe { (*slot.value.get()).assume_init_read() };

        // Compute new value from old
        let new_value = f(&old_value);

        // Write new value (slot is still marked occupied)
        unsafe {
            (*slot.value.get()).write(new_value);
        }

        Some(old_value)
    }

    // =========================================================================
    // Modify API
    // =========================================================================

    /// Modifies the value in place if valid.
    ///
    /// Returns `&self` for chaining. If the entry is invalid or borrowed,
    /// the closure is not called and the chain continues.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let slab = BoundedSlab::with_capacity(16);
    /// let entry = slab.insert(0u64).unwrap();
    ///
    /// entry
    ///     .and_modify(|v| *v += 1)
    ///     .and_modify(|v| *v *= 2);
    ///
    /// assert_eq!(*entry.get(), 2);
    /// ```
    pub fn and_modify<F>(&self, f: F) -> &Self
    where
        F: FnOnce(&mut T),
    {
        if let Some(mut guard) = self.try_get_mut() {
            f(&mut *guard);
        }
        self
    }

    // =========================================================================
    // Take API (extract value + get VacantEntry)
    // =========================================================================

    /// Extracts the value, returning it with a [`VacantEntry`] for the slot.
    ///
    /// Unlike [`remove`](Self::remove), this keeps the slot reserved. The
    /// `VacantEntry` can be used to insert a new value into the same slot,
    /// or dropped to return the slot to the freelist.
    ///
    /// # Panics
    ///
    /// Panics if the slab was dropped, slot is vacant, or currently borrowed.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let slab = BoundedSlab::with_capacity(16);
    /// let entry = slab.insert(42u64).unwrap();
    /// let key = entry.key();
    ///
    /// // Take the value but keep the slot
    /// let (old_value, vacant) = entry.take();
    /// assert_eq!(old_value, 42);
    /// assert_eq!(vacant.key(), key);  // Same slot
    ///
    /// // Insert a new value into the same slot
    /// let new_entry = vacant.insert(100);
    /// assert_eq!(new_entry.key(), key);
    /// ```
    pub fn take(self) -> (T, VacantEntry<T>) {
        self.try_take().expect("entry is invalid or borrowed")
    }

    /// Extracts the value if valid, returning it with a [`VacantEntry`].
    ///
    /// Returns `None` if the slab has been dropped, the slot is vacant,
    /// or the slot is currently borrowed.
    pub fn try_take(self) -> Option<(T, VacantEntry<T>)> {
        let _inner = self.slab.upgrade()?;
        let slot = unsafe { &*self.slot_ptr };

        if !slot.is_available() {
            return None;
        }

        // Read the value
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Mark slot as vacant but don't update freelist
        // (VacantEntry will handle that on drop or insert will mark it occupied)
        // Actually, we need to leave slot in a state where VacantEntry::insert works.
        // Looking at VacantEntry::insert, it expects to call slot.set_occupied().
        // And VacantEntry::drop expects to call slot.set_vacant(free_head).
        // So we should NOT mark it vacant here - let VacantEntry handle it.
        // But we need the slot to NOT be considered occupied for the len count.
        // Actually the len is already correct - we're not changing len here,
        // and VacantEntry::insert doesn't change len (it was already incremented).
        // But wait - the slab's len was incremented when this slot was first inserted.
        // If we take without touching len, then VacantEntry::drop would decrement len,
        // which is correct if the user drops without inserting.
        // And VacantEntry::insert doesn't touch len, which is also correct.
        // So we just need to NOT update len here.

        // Create VacantEntry (which will handle cleanup on drop or fill on insert)
        let vacant = VacantEntry {
            slab: self.slab.clone(),
            slot_ptr: self.slot_ptr,
            index: self.index,
            _marker: PhantomData,
        };

        Some((value, vacant))
    }

    /// Extracts the value without checks, returning it with a [`VacantEntry`].
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - Slab is still alive
    /// - Slot is occupied
    /// - No active borrows
    pub unsafe fn take_unchecked(self) -> (T, VacantEntry<T>) {
        let slot = unsafe { &*self.slot_ptr };

        // Read the value
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Create VacantEntry
        let vacant = VacantEntry {
            slab: self.slab.clone(),
            slot_ptr: self.slot_ptr,
            index: self.index,
            _marker: PhantomData,
        };

        (value, vacant)
    }
}

impl<T> Clone for Entry<T> {
    fn clone(&self) -> Self {
        Self {
            slab: self.slab.clone(),
            slot_ptr: self.slot_ptr,
            index: self.index,
        }
    }
}

impl<T> PartialEq for Entry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.slot_ptr == other.slot_ptr
    }
}

impl<T> Eq for Entry<T> {}

impl<T> fmt::Debug for Entry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("index", &self.index)
            .field("alive", &self.is_alive())
            .finish()
    }
}

// =============================================================================
// Ref
// =============================================================================

/// An immutable borrow of a value in the slab.
pub struct Ref<T> {
    pub(crate) _slab: Rc<BoundedSlabInner<T>>,
    pub(crate) slot_ptr: *const SlotCell<T>,
}

impl<T> Deref for Ref<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { (*self.slot_ptr).value_ref() }
    }
}

impl<T> Drop for Ref<T> {
    fn drop(&mut self) {
        unsafe { (*self.slot_ptr).clear_borrowed() };
    }
}

impl<T: fmt::Debug> fmt::Debug for Ref<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for Ref<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

// =============================================================================
// RefMut
// =============================================================================

/// A mutable borrow of a value in the slab.
pub struct RefMut<T> {
    pub(crate) _slab: Rc<BoundedSlabInner<T>>,
    pub(crate) slot_ptr: *const SlotCell<T>,
}

impl<T> Deref for RefMut<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { (*self.slot_ptr).value_ref() }
    }
}

impl<T> DerefMut for RefMut<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { (*self.slot_ptr).value_mut() }
    }
}

impl<T> Drop for RefMut<T> {
    fn drop(&mut self) {
        unsafe { (*self.slot_ptr).clear_borrowed() };
    }
}

impl<T: fmt::Debug> fmt::Debug for RefMut<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for RefMut<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_capacity_basic() {
        let slab: BoundedSlab<u64> = BoundedSlab::with_capacity(100);
        assert_eq!(slab.capacity(), 100);
        assert_eq!(slab.len(), 0);
        assert!(slab.is_empty());
        assert!(!slab.is_full());
    }

    #[test]
    #[should_panic(expected = "capacity must be non-zero")]
    fn with_capacity_zero_panics() {
        let _: BoundedSlab<u64> = BoundedSlab::with_capacity(0);
    }

    #[test]
    fn insert_get_remove() {
        let slab = BoundedSlab::with_capacity(16);

        let entry = slab.insert(42u64).unwrap();
        assert_eq!(slab.len(), 1);
        assert_eq!(*entry.get(), 42);

        let removed = entry.remove();
        assert_eq!(removed, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn insert_full_returns_error() {
        let slab = BoundedSlab::with_capacity(2);

        slab.insert(1u64).unwrap();
        slab.insert(2u64).unwrap();

        let result = slab.insert(3u64);
        assert!(result.is_err());
        // Verify we can recover the value
        assert_eq!(result.unwrap_err().into_inner(), 3u64);
        assert!(slab.is_full());
    }

    #[test]
    fn entry_key_roundtrip() {
        let slab = BoundedSlab::with_capacity(16);

        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        let entry2 = slab.entry(key).unwrap();
        assert_eq!(*entry2.get(), 42);
    }

    #[test]
    fn double_borrow_fails() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry2 = entry.clone();

        let _r1 = entry.try_get();
        assert!(entry2.try_get().is_none());
    }

    #[test]
    fn borrow_released_on_drop() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry2 = entry.clone();

        {
            let _r1 = entry.try_get();
        }

        assert!(entry2.try_get().is_some());
    }

    #[test]
    fn insert_with_self_referential() {
        struct Node {
            self_ref: Entry<Node>,
            data: u64,
        }

        let slab = BoundedSlab::with_capacity(16);

        let entry = slab
            .insert_with(|e| Node {
                self_ref: e.clone(),
                data: 42,
            })
            .unwrap();

        let node = entry.get();
        assert_eq!(node.data, 42);
        assert_eq!(node.self_ref.key(), entry.key());
    }

    #[test]
    fn clear_empties_slab() {
        let slab = BoundedSlab::with_capacity(16);

        let e1 = slab.insert(1u64).unwrap();
        let e2 = slab.insert(2u64).unwrap();

        slab.clear();

        assert_eq!(slab.len(), 0);
        assert!(e1.try_get().is_none());
        assert!(e2.try_get().is_none());
    }

    #[test]
    fn stress_insert_remove_cycle() {
        let slab = BoundedSlab::with_capacity(1);

        for i in 0..10_000u64 {
            let entry = slab.insert(i).unwrap();
            assert_eq!(*entry.get(), i);
            assert_eq!(entry.remove(), i);
        }
    }

    #[test]
    fn slab_remove_fast_path() {
        let slab = BoundedSlab::with_capacity(16);

        let entry = slab.insert(42u64).unwrap();
        assert_eq!(slab.len(), 1);

        let value = slab.remove(entry);
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn slab_remove_fast_path_reuses_slot() {
        let slab = BoundedSlab::with_capacity(1);

        for i in 0..1000u64 {
            let entry = slab.insert(i).unwrap();
            let value = slab.remove(entry);
            assert_eq!(value, i);
        }
    }

    #[test]
    fn key_based_access() {
        let slab = BoundedSlab::with_capacity(16);

        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        assert!(slab.contains_key(key));

        // Safe tracked access returns Ref guard
        {
            let guard = slab.get(key).unwrap();
            assert_eq!(*guard, 42);
        }

        // Unsafe untracked access via UntrackedAccessor for indexing
        unsafe {
            let accessor = slab.untracked();
            assert_eq!(accessor[key], 42);
        }

        let removed = slab.remove_by_key(key);
        assert_eq!(removed, 42);
        assert!(!slab.contains_key(key));
    }

    #[test]
    fn get_untracked_basic() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        // Slab-level untracked
        unsafe {
            assert_eq!(slab.get_untracked(key), Some(&42));
            assert_eq!(slab.get_untracked_mut(key), Some(&mut 42));
        }

        // Entry-level untracked
        unsafe {
            assert_eq!(entry.get_untracked(), Some(&42));
            assert_eq!(entry.get_untracked_mut(), Some(&mut 42));
        }
    }

    #[test]
    fn untracked_accessor_basic() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        unsafe {
            let accessor = slab.untracked();
            assert_eq!(accessor[key], 42);
        }

        unsafe {
            let mut accessor = slab.untracked();
            accessor[key] = 100;
        }

        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn tracked_get_blocks_double_borrow() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        // Hold a tracked borrow via slab.get()
        let _guard = slab.get(key).unwrap();

        // Entry access should fail (slot is borrowed)
        assert!(entry.try_get().is_none());

        // Another slab.get() should also fail
        assert!(slab.get(key).is_none());
    }

    #[test]
    fn entry_is_valid() {
        let slab = BoundedSlab::with_capacity(16);

        let entry = slab.insert(42u64).unwrap();
        assert!(entry.is_valid());

        // Still valid even when borrowed
        let _r = entry.get();
        let entry2 = entry.clone();
        assert!(entry2.is_valid());

        drop(_r);

        // Invalid after remove
        entry.remove();
        assert!(!entry2.is_valid());
    }

    #[test]
    fn entry_equality() {
        let slab = BoundedSlab::with_capacity(16);

        let e1 = slab.insert(1u64).unwrap();
        let e2 = slab.insert(2u64).unwrap();
        let e1_clone = e1.clone();

        assert_eq!(e1, e1_clone);
        assert_ne!(e1, e2);
    }

    #[test]
    fn insert_unchecked_basic() {
        let slab = BoundedSlab::with_capacity(16);

        let entry = unsafe { slab.insert_unchecked(42u64) };
        assert_eq!(slab.len(), 1);
        assert_eq!(*entry.get(), 42);
    }

    #[test]
    fn remove_unchecked_by_key_basic() {
        let slab = BoundedSlab::with_capacity(16);

        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        let value = unsafe { slab.remove_unchecked_by_key(key) };
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn insert_unchecked_stress() {
        let slab = BoundedSlab::with_capacity(1000);

        for i in 0..1000u64 {
            let entry = unsafe { slab.insert_unchecked(i) };
            assert_eq!(*entry.get(), i);
        }

        assert!(slab.is_full());
    }

    #[test]
    fn remove_unchecked_by_key_stress() {
        let slab = BoundedSlab::with_capacity(1000);

        let keys: Vec<Key> = (0..1000u64)
            .map(|i| slab.insert(i).unwrap().key())
            .collect();

        for (i, key) in keys.into_iter().enumerate() {
            let value = unsafe { slab.remove_unchecked_by_key(key) };
            assert_eq!(value, i as u64);
        }

        assert!(slab.is_empty());
    }

    #[test]
    fn try_remove_success() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();

        let value = slab.try_remove(entry);
        assert_eq!(value, Some(42));
        assert!(slab.is_empty());
    }

    #[test]
    fn try_remove_vacant_returns_none() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry_clone = entry.clone();

        // Remove via one handle
        slab.remove(entry);

        // Try to remove via the other - should return None
        let result = slab.try_remove(entry_clone);
        assert!(result.is_none());
    }

    #[test]
    fn try_remove_borrowed_returns_none() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry_clone = entry.clone();

        // Hold a borrow
        let _guard = entry.get();

        // Try to remove - should return None because borrowed
        let result = slab.try_remove(entry_clone);
        assert!(result.is_none());
    }

    #[test]
    fn try_remove_by_key_success() {
        let slab = BoundedSlab::with_capacity(16);
        let key = slab.insert(42u64).unwrap().key();

        let value = slab.try_remove_by_key(key);
        assert_eq!(value, Some(42));
        assert!(slab.is_empty());
    }

    #[test]
    fn try_remove_by_key_invalid_returns_none() {
        let slab = BoundedSlab::<u64>::with_capacity(16);

        // Invalid key (out of bounds)
        let result = slab.try_remove_by_key(Key::from_raw(100));
        assert!(result.is_none());

        // Valid index but vacant
        let key = slab.insert(42u64).unwrap().key();
        slab.remove_by_key(key);
        let result = slab.try_remove_by_key(key);
        assert!(result.is_none());
    }

    #[test]
    fn vacant_entry_insert() {
        let slab = BoundedSlab::with_capacity(16);

        let vacant = slab.vacant_entry().unwrap();
        let key = vacant.key();
        assert_eq!(slab.len(), 1); // Slot is reserved

        let entry = vacant.insert(42u64);
        assert_eq!(slab.len(), 1);
        assert_eq!(*entry.get(), 42);
        assert_eq!(entry.key(), key);
    }

    #[test]
    fn vacant_entry_drop_returns_slot() {
        let slab = BoundedSlab::with_capacity(16);

        {
            let vacant = slab.vacant_entry().unwrap();
            assert_eq!(slab.len(), 1);
            let _key = vacant.key();
            // Drop without insert
        }

        // Slot should be returned to freelist
        assert_eq!(slab.len(), 0);

        // Should be able to insert again
        let entry = slab.insert(42u64).unwrap();
        assert_eq!(slab.len(), 1);
        assert_eq!(*entry.get(), 42);
    }

    #[test]
    fn vacant_entry_full_returns_error() {
        let slab = BoundedSlab::with_capacity(2);

        slab.insert(1u64).unwrap();
        slab.insert(2u64).unwrap();

        assert!(slab.vacant_entry().is_err());
    }

    #[test]
    fn vacant_entry_key_matches_final_entry() {
        let slab = BoundedSlab::with_capacity(16);

        let vacant = slab.vacant_entry().unwrap();
        let expected_key = vacant.key();

        let entry = vacant.insert(42u64);
        assert_eq!(entry.key(), expected_key);

        // Key should work with slab methods
        assert_eq!(*slab.get(expected_key).unwrap(), 42);
    }

    #[test]
    fn vacant_entry_reuses_freed_slot() {
        let slab = BoundedSlab::with_capacity(16);

        // Insert and remove to put slot on freelist
        let key1 = slab.insert(1u64).unwrap().key();
        slab.remove_by_key(key1);

        // Vacant entry should get the same slot (LIFO)
        let vacant = slab.vacant_entry().unwrap();
        assert_eq!(vacant.key(), key1);

        let entry = vacant.insert(2u64);
        assert_eq!(entry.key(), key1);
    }

    // =========================================================================
    // Entry::replace tests
    // =========================================================================

    #[test]
    fn entry_replace_basic() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();

        let old = entry.replace(100);
        assert_eq!(old, 42);
        assert_eq!(*entry.get(), 100);
        assert_eq!(slab.len(), 1); // Still occupied
    }

    #[test]
    fn entry_try_replace_success() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();

        let old = entry.try_replace(100);
        assert_eq!(old, Some(42));
        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn entry_try_replace_vacant_returns_none() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry_clone = entry.clone();

        entry.remove();

        let result = entry_clone.try_replace(100);
        assert!(result.is_none());
    }

    #[test]
    fn entry_try_replace_borrowed_returns_none() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry_clone = entry.clone();

        let _guard = entry.get();

        let result = entry_clone.try_replace(100);
        assert!(result.is_none());
    }

    #[test]
    fn entry_replace_with_closure() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();

        let old = entry.replace_with(|v| v * 2);
        assert_eq!(old, 42);
        assert_eq!(*entry.get(), 84);
    }

    #[test]
    fn entry_try_replace_with_closure() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();

        let old = entry.try_replace_with(|v| v + 10);
        assert_eq!(old, Some(42));
        assert_eq!(*entry.get(), 52);
    }

    // =========================================================================
    // Entry::and_modify tests
    // =========================================================================

    #[test]
    fn entry_and_modify_basic() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(0u64).unwrap();

        entry.and_modify(|v| *v += 10);
        assert_eq!(*entry.get(), 10);
    }

    #[test]
    fn entry_and_modify_chaining() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(1u64).unwrap();

        entry
            .and_modify(|v| *v += 1) // 2
            .and_modify(|v| *v *= 3) // 6
            .and_modify(|v| *v -= 2); // 4

        assert_eq!(*entry.get(), 4);
    }

    #[test]
    fn entry_and_modify_invalid_skips() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry_clone = entry.clone();

        entry.remove();

        // Should not panic, just skip the modification
        entry_clone.and_modify(|v| *v = 100);
    }

    #[test]
    fn entry_and_modify_borrowed_skips() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry_clone = entry.clone();

        let _guard = entry.get();

        // Should not panic, just skip
        entry_clone.and_modify(|v| *v = 100);
    }

    // =========================================================================
    // Entry::take tests
    // =========================================================================

    #[test]
    fn entry_take_basic() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        let (value, vacant) = entry.take();
        assert_eq!(value, 42);
        assert_eq!(vacant.key(), key);
        assert_eq!(slab.len(), 1); // Still reserved
    }

    #[test]
    fn entry_take_then_insert() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        let (old_value, vacant) = entry.take();
        assert_eq!(old_value, 42);

        let new_entry = vacant.insert(100);
        assert_eq!(new_entry.key(), key); // Same slot
        assert_eq!(*new_entry.get(), 100);
        assert_eq!(slab.len(), 1);
    }

    #[test]
    fn entry_take_then_drop() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();

        let (value, vacant) = entry.take();
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 1); // Reserved

        drop(vacant);
        assert_eq!(slab.len(), 0); // Now freed
    }

    #[test]
    fn entry_try_take_success() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        let result = entry.try_take();
        assert!(result.is_some());

        let (value, vacant) = result.unwrap();
        assert_eq!(value, 42);
        assert_eq!(vacant.key(), key);
    }

    #[test]
    fn entry_try_take_vacant_returns_none() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry_clone = entry.clone();

        entry.remove();

        let result = entry_clone.try_take();
        assert!(result.is_none());
    }

    #[test]
    fn entry_try_take_borrowed_returns_none() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let entry_clone = entry.clone();

        let _guard = entry.get();

        let result = entry_clone.try_take();
        assert!(result.is_none());
    }

    #[test]
    fn entry_take_unchecked_basic() {
        let slab = BoundedSlab::with_capacity(16);
        let entry = slab.insert(42u64).unwrap();
        let key = entry.key();

        let (value, vacant) = unsafe { entry.take_unchecked() };
        assert_eq!(value, 42);
        assert_eq!(vacant.key(), key);

        // Clean up
        let new_entry = vacant.insert(0);
        new_entry.remove();
    }
}
