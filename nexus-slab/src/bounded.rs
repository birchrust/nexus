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

use std::cell::{Cell, UnsafeCell};
use std::fmt;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::{Deref, DerefMut, Index, IndexMut};
use std::pin::Pin;
use std::rc::{Rc, Weak};

use crate::Key;

// =============================================================================
// Constants
// =============================================================================

/// Bit 31: Vacant flag (1 = vacant)
pub(crate) const VACANT_BIT: u32 = 1 << 31;

/// Bit 30: Borrowed flag (1 = borrowed)
pub(crate) const BORROWED_BIT: u32 = 1 << 30;

/// Mask for next_free index (bits 0-29)
pub(crate) const INDEX_MASK: u32 = (1 << 30) - 1;

/// Sentinel for end of freelist (~1 billion max capacity)
pub(crate) const SLOT_NONE: u32 = INDEX_MASK;

// =============================================================================
// SlotCell
// =============================================================================

/// Internal slot storage with tag for state tracking.
///
/// Tag encoding (32-bit):
/// - Bit 31: Vacant flag (1 = vacant, 0 = occupied)
/// - Bit 30: Borrowed flag (1 = borrowed, 0 = available) - only when occupied
/// - Bits 0-29: When vacant, next free slot index
#[repr(C)]
pub(crate) struct SlotCell<T> {
    tag: Cell<u32>,
    pub(crate) value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> SlotCell<T> {
    pub(crate) fn new_vacant(next_free: u32) -> Self {
        Self {
            tag: Cell::new(VACANT_BIT | (next_free & INDEX_MASK)),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    #[inline]
    pub(crate) fn is_vacant(&self) -> bool {
        self.tag.get() & VACANT_BIT != 0
    }

    #[inline]
    pub(crate) fn is_occupied(&self) -> bool {
        !self.is_vacant()
    }

    #[inline]
    pub(crate) fn is_borrowed(&self) -> bool {
        self.tag.get() == BORROWED_BIT
    }

    #[inline]
    pub(crate) fn next_free(&self) -> u32 {
        debug_assert!(self.is_vacant(), "next_free called on occupied slot");
        self.tag.get() & INDEX_MASK
    }

    #[inline]
    pub(crate) fn set_occupied(&self) {
        self.tag.set(0);
    }

    #[inline]
    pub(crate) fn set_vacant(&self, next_free: u32) {
        self.tag.set(VACANT_BIT | (next_free & INDEX_MASK));
    }

    #[inline]
    pub(crate) fn set_borrowed(&self) {
        debug_assert!(self.is_occupied(), "set_borrowed on vacant slot");
        debug_assert!(!self.is_borrowed(), "already borrowed");
        self.tag.set(BORROWED_BIT);
    }

    #[inline]
    pub(crate) fn clear_borrowed(&self) {
        debug_assert!(self.is_borrowed(), "clear_borrowed on non-borrowed slot");
        self.tag.set(0);
    }

    /// # Safety
    /// Slot must be occupied.
    #[inline]
    pub(crate) unsafe fn value_ref(&self) -> &T {
        unsafe { (*self.value.get()).assume_init_ref() }
    }

    /// # Safety
    /// Slot must be occupied and caller must have exclusive access.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn value_mut(&self) -> &mut T {
        unsafe { (*self.value.get()).assume_init_mut() }
    }
}

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
    /// Returns `None` if the slab is full.
    pub fn insert(&self, value: T) -> Option<Entry<T>> {
        let inner = &*self.inner;
        let free_head = inner.free_head.get();

        if free_head == SLOT_NONE {
            return None;
        }

        let slot = inner.slot(free_head);
        let next_free = slot.next_free();

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        Some(Entry {
            slab: Rc::downgrade(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
            index: free_head,
        })
    }

    /// Inserts with access to the Entry before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own Entry.
    pub fn insert_with<F>(&self, f: F) -> Option<Entry<T>>
    where
        F: FnOnce(Entry<T>) -> T,
    {
        let inner = &*self.inner;
        let free_head = inner.free_head.get();

        if free_head == SLOT_NONE {
            return None;
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

        Some(entry)
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
        let slot = unsafe { &*entry.slot_ptr };

        debug_assert!(!slot.is_vacant(), "remove called on vacant slot");
        debug_assert!(!slot.is_borrowed(), "remove called on borrowed slot");

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let free_head = self.inner.free_head.get();
        slot.set_vacant(free_head);
        self.inner.free_head.set(entry.index);
        self.inner.len.set(self.inner.len.get() - 1);

        value
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
    pub fn contains(&self, key: Key) -> bool {
        let index = key.index();
        if index >= self.inner.capacity {
            return false;
        }
        self.inner.slot(index).is_occupied()
    }

    /// Returns a reference to the value at `key`.
    #[inline]
    pub fn get(&self, key: Key) -> Option<&T> {
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

    /// Returns a mutable reference to the value at `key`.
    ///
    /// # Safety Note
    ///
    /// This uses interior mutability. The caller must ensure no other
    /// references to this slot exist.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub fn get_mut(&self, key: Key) -> Option<&mut T> {
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

    /// Returns a reference without bounds or occupancy checks.
    ///
    /// # Safety
    ///
    /// The key must be valid and the slot must be occupied.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        unsafe { self.inner.slot(key.index()).value_ref() }
    }

    /// Returns a mutable reference without bounds or occupancy checks.
    ///
    /// # Safety
    ///
    /// The key must be valid, the slot must be occupied, and the caller
    /// must ensure exclusive access.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_unchecked_mut(&self, key: Key) -> &mut T {
        unsafe { self.inner.slot(key.index()).value_mut() }
    }

    /// Removes and returns the value at `key`.
    ///
    /// # Panics
    ///
    /// Panics if the key is invalid or the slot is vacant.
    #[inline]
    pub fn remove_by_key(&self, key: Key) -> T {
        let index = key.index();
        assert!(index < self.inner.capacity, "key out of bounds");

        let slot = self.inner.slot(index);
        assert!(!slot.is_vacant(), "remove called on vacant slot");

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

impl<T> Index<Key> for BoundedSlab<T> {
    type Output = T;

    #[inline]
    fn index(&self, key: Key) -> &T {
        self.get(key).expect("invalid key")
    }
}

impl<T> IndexMut<Key> for BoundedSlab<T> {
    #[inline]
    fn index_mut(&mut self, key: Key) -> &mut T {
        self.get_mut(key).expect("invalid key")
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

        if slot.is_vacant() || slot.is_borrowed() {
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

        if slot.is_vacant() || slot.is_borrowed() {
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

        if slot.is_vacant() || slot.is_borrowed() {
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
    // Unchecked API
    // =========================================================================

    /// Direct read without any checks.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - Slab is still alive
    /// - Slot is occupied
    /// - No concurrent mutable access
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
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_mut_unchecked(&self) -> &mut T {
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
    /// Same requirements as [`get_mut_unchecked`](Self::get_mut_unchecked).
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_pinned_mut_unchecked(&self) -> Pin<&mut T> {
        unsafe { Pin::new_unchecked(self.get_mut_unchecked()) }
    }

    /// Remove without any checks.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - Slab is still alive
    /// - Slot is occupied
    /// - No active borrows
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
    _slab: Rc<BoundedSlabInner<T>>,
    slot_ptr: *const SlotCell<T>,
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
    _slab: Rc<BoundedSlabInner<T>>,
    slot_ptr: *const SlotCell<T>,
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
    fn insert_full_returns_none() {
        let slab = BoundedSlab::with_capacity(2);

        slab.insert(1u64).unwrap();
        slab.insert(2u64).unwrap();

        assert!(slab.insert(3u64).is_none());
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

        assert!(slab.contains(key));
        assert_eq!(slab.get(key), Some(&42));
        assert_eq!(slab[key], 42);

        let removed = slab.remove_by_key(key);
        assert_eq!(removed, 42);
        assert!(!slab.contains(key));
    }
}
