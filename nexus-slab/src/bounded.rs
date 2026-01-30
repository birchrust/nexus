//! Fixed-capacity slab allocator with RAII Entry-based access.
//!
//! [`Slab`] provides a pre-allocated, leaked slab where all memory is
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
//! use nexus_slab::bounded::Slab;
//!
//! let slab = Slab::with_capacity(1024);
//!
//! // RAII entry - slot freed when entry drops
//! {
//!     let entry = slab.try_insert("hello").unwrap();
//!     assert_eq!(*entry.get(), "hello");
//! } // entry drops, slot freed
//!
//! // Forget to keep data alive
//! let entry = slab.try_insert("world").unwrap();
//! let key = entry.forget(); // data stays, returns Key
//!
//! // Access via key (unsafe - caller guarantees key validity)
//! // SAFETY: key was just returned from forget(), slot is occupied
//! assert_eq!(*unsafe { slab.get_by_key(key) }, "world");
//! ```
//!
//! # Self-Referential Patterns
//!
//! ```
//! use nexus_slab::{bounded::Slab, Key};
//!
//! struct Node {
//!     self_key: Key,
//!     data: u64,
//! }
//!
//! let slab: Slab<Node> = Slab::with_capacity(16);
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
use std::pin::Pin;

use crate::shared::{SLOT_NONE, SlotCell};
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
        assert!(capacity < SLOT_NONE, "capacity exceeds maximum");

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
// Entry
// =============================================================================

/// RAII handle to an occupied slot in a bounded [`Slab`].
///
/// When dropped, the slot is deallocated and returned to the freelist.
/// Use [`forget()`](Self::forget) to keep the data alive without the entry.
///
/// # Size
///
/// 16 bytes: slot pointer (8) + inner pointer (8).
#[must_use = "dropping Entry deallocates the slot"]
pub struct Entry<T> {
    slot: *mut SlotCell<T>,
    inner: *mut BoundedSlabInner<T>,
}

impl<T> Entry<T> {
    /// Creates a new entry.
    #[inline]
    pub(crate) fn new(slot: *mut SlotCell<T>, inner: *mut BoundedSlabInner<T>) -> Self {
        Self { slot, inner }
    }

    #[inline]
    fn slot(&self) -> &SlotCell<T> {
        // SAFETY: Entry holds a valid slot pointer
        unsafe { &*self.slot }
    }

    #[inline]
    fn inner(&self) -> &BoundedSlabInner<T> {
        // SAFETY: Entry holds a valid inner pointer (leaked)
        unsafe { &*self.inner }
    }

    /// Extracts the value, returning it with a [`VacantEntry`] for the slot.
    ///
    /// Unlike drop, this keeps the slot reserved. The VacantEntry can be used
    /// to insert a new value into the same slot, or dropped to return the slot
    /// to the freelist.
    pub fn take(self) -> (T, VacantEntry<T>) {
        let slot = self.slot();
        let key = self.key();

        // SAFETY: Entry owns the slot, so it's valid
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let vacant = VacantEntry {
            inner: self.inner,
            key,
            consumed: false,
            _marker: PhantomData,
        };

        // Don't run Entry's Drop (which would deallocate)
        std::mem::forget(self);

        (value, vacant)
    }
}

// Core Entry methods as inherent (no trait import needed)
impl<T> Entry<T> {
    /// Returns the key for this entry.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.slot().key_from_stamp())
    }

    /// Gives up ownership of this entry, keeping the data alive and returning its key.
    ///
    /// After calling `forget()`, the slot remains occupied but has no Entry owner.
    /// Access the data via key-based methods (which are unsafe) or create a new
    /// Entry via [`Slab::entry()`].
    ///
    /// Named after `std::mem::forget` - you're telling the Entry to not run its
    /// destructor (which would deallocate the slot).
    #[inline]
    pub fn forget(self) -> Key {
        let key = self.key();
        std::mem::forget(self);
        key
    }

    /// Returns a reference to the value.
    ///
    /// Entry ownership guarantees the slot is valid.
    #[inline]
    pub fn get(&self) -> &T {
        // SAFETY: Entry owns the slot. SlotCell is repr(C): [stamp: 8][value: T]
        // Go directly to value at offset 8, bypassing abstraction chain.
        unsafe { &*((self.slot as *const u8).add(8) as *const T) }
    }

    /// Returns a mutable reference to the value.
    ///
    /// Requires `&mut Entry` to ensure exclusive access.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: Entry owns the slot, &mut self ensures exclusivity.
        // SlotCell is repr(C): [stamp: 8][value: T]
        unsafe { &mut *((self.slot as *mut u8).add(8) as *mut T) }
    }

    /// Replaces the value, returning the old one.
    #[inline]
    pub fn replace(&mut self, value: T) -> T {
        // SAFETY: Entry owns the slot. SlotCell is repr(C): [stamp: 8][value: T]
        // Direct pointer access to value at offset 8.
        let value_ptr = unsafe { (self.slot as *mut u8).add(8) as *mut T };
        let old = unsafe { value_ptr.read() };
        unsafe { value_ptr.write(value) };
        old
    }

    /// Modifies the value in place. Returns self for chaining.
    #[inline]
    pub fn and_modify<F: FnOnce(&mut T)>(&mut self, f: F) -> &mut Self {
        f(self.get_mut());
        self
    }

    /// Returns `true` if the slot is still occupied.
    ///
    /// This should always return `true` for a properly-used Entry.
    /// Returns `false` only if the slot was incorrectly deallocated
    /// via unsafe key-based methods while this Entry existed.
    ///
    /// Useful for debug assertions to catch API misuse.
    #[inline]
    pub fn is_valid(&self) -> bool {
        // SAFETY: SlotCell is repr(C): [stamp: 8][value: T]
        // Read stamp directly at offset 0. Occupied = VACANT_BIT not set.
        let stamp = unsafe { *(self.slot as *const u64) };
        stamp & crate::shared::VACANT_BIT == 0
    }

    /// Returns a pinned reference to the value.
    ///
    /// This is safe because slab slots have stable addresses—the slab
    /// is leaked and never reallocates.
    #[inline]
    pub fn pin(&self) -> Pin<&T> {
        // SAFETY: Slot memory is leaked and never moves
        unsafe { Pin::new_unchecked(self.get()) }
    }

    /// Returns a pinned mutable reference to the value.
    ///
    /// This is safe because slab slots have stable addresses—the slab
    /// is leaked and never reallocates.
    #[inline]
    pub fn pin_mut(&mut self) -> Pin<&mut T> {
        // SAFETY: Slot memory is leaked and never moves
        unsafe { Pin::new_unchecked(self.get_mut()) }
    }

    /// Removes the entry, returning the value.
    ///
    /// The slot is returned to the freelist.
    #[inline]
    pub fn remove(self) -> T {
        let slot = self.slot();
        let slot_index = slot.key_from_stamp();

        // SAFETY: Entry owns the slot
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Return slot to freelist
        let inner = self.inner();
        let free_head = inner.free_head.get();
        slot.set_vacant(free_head);
        inner.free_head.set(slot_index);
        inner.len.set(inner.len.get() - 1);

        // Don't run Drop (we already handled deallocation)
        std::mem::forget(self);

        value
    }
}

impl<T> Drop for Entry<T> {
    fn drop(&mut self) {
        let slot = self.slot();

        // SAFETY: Entry is sole owner (!Clone), so if Drop runs, slot is occupied
        unsafe {
            std::ptr::drop_in_place((*slot.value.get()).as_mut_ptr());
        }

        // Return slot to freelist
        let inner = self.inner();
        let slot_index = slot.key_from_stamp();
        let free_head = inner.free_head.get();
        slot.set_vacant(free_head);
        inner.free_head.set(slot_index);
        inner.len.set(inner.len.get() - 1);
    }
}

impl<T> fmt::Debug for Entry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry").field("key", &self.key()).finish()
    }
}

impl<T> std::ops::Deref for Entry<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> std::ops::DerefMut for Entry<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl<T> AsRef<T> for Entry<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self.get()
    }
}

impl<T> AsMut<T> for Entry<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T> std::borrow::Borrow<T> for Entry<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self.get()
    }
}

impl<T> std::borrow::BorrowMut<T> for Entry<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T: fmt::Display> fmt::Display for Entry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl<T> fmt::Pointer for Entry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.slot, f)
    }
}

// =============================================================================
// Slab
// =============================================================================

/// A fixed-capacity slab allocator with RAII Entry-based access.
///
/// Created via [`with_capacity`](Self::with_capacity) or the [`Builder`](crate::Builder) API.
/// The slab is leaked and lives for `'static`.
///
/// # Thread Safety
///
/// `Slab` is `!Send` and `!Sync` - it uses raw pointers internally.
/// The slab must only be used from the thread that created it.
///
/// # Example
///
/// ```
/// use nexus_slab::bounded::Slab;
///
/// let slab = Slab::with_capacity(1024);
///
/// let entry = slab.try_insert(42).unwrap();
/// assert_eq!(*entry.get(), 42);
/// // entry drops, slot freed
/// ```
#[derive(Clone, Copy)]
pub struct Slab<T> {
    pub(crate) ptr: *mut BoundedSlabInner<T>,
    _marker: PhantomData<*mut ()>, // Ensures !Send + !Sync
}

impl<T> Slab<T> {
    #[inline]
    fn inner(&self) -> &BoundedSlabInner<T> {
        // SAFETY: ptr is valid for 'static (leaked)
        unsafe { &*self.ptr }
    }

    /// Creates a slab with the given capacity.
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
    /// use nexus_slab::bounded::Slab;
    ///
    /// let slab = Slab::<String>::with_capacity(1024);
    /// assert_eq!(slab.capacity(), 1024);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        let inner = Box::new(BoundedSlabInner::with_capacity(capacity as u32));
        let inner_ptr = Box::into_raw(inner);

        Self {
            ptr: inner_ptr,
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
    /// deallocated. Use [`Entry::forget()`] to keep the data alive.
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
        let next_free = slot.claim_next_free(); // 1 stamp read

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_key_occupied(free_head); // 1 stamp write

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Ok(Entry::new(slot_ptr, self.ptr))
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
        let next_free = slot.claim_next_free(); // 1 stamp read

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        // Store key in stamp BEFORE creating Entry (so Entry::key() works)
        // This requires read-modify-write since we preserve VACANT_BIT temporarily
        slot.set_key(free_head);

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();

        // Create entry (slot not yet occupied, but key is readable from stamp)
        let entry = Entry::new(slot_ptr, self.ptr);

        // Call closure to get value
        let value = f(&entry);

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_key_occupied(free_head); // 1 stamp write (overwrites, don't need read-modify-write)

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
        let next_free = slot.claim_next_free(); // 1 stamp read

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        // Store key in stamp - VacantEntry::insert will later call set_key_occupied
        slot.set_key(free_head);

        Ok(VacantEntry {
            inner: self.ptr,
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

        // Key is already in slot's stamp from when it was inserted
        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Some(Entry::new(slot_ptr, self.ptr))
    }

    /// Removes a value by key.
    ///
    /// Use this when you have a forgotten key and want to deallocate.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No Entry may exist for this slot (would become dangling)
    #[inline]
    pub unsafe fn remove_by_key(&self, key: Key) -> T {
        let index = key.index();
        let inner = self.inner();
        let slot = inner.slot(index);

        // SAFETY: Caller guarantees slot is occupied
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let free_head = inner.free_head.get();
        slot.set_vacant(free_head);
        inner.free_head.set(index);
        inner.len.set(inner.len.get() - 1);

        value
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
    // Unsafe key-based access
    // =========================================================================

    /// Returns a reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No Entry may have exclusive (`&mut`) access to this slot
    /// - Caller must ensure no aliasing violations
    #[inline]
    pub unsafe fn get_by_key(&self, key: Key) -> &T {
        let slot = self.inner().slot(key.index());
        unsafe { slot.value_ref() }
    }

    /// Returns a mutable reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No other references (Entry-based or key-based) may exist to this slot
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_by_key_mut(&self, key: Key) -> &mut T {
        let slot = self.inner().slot(key.index());
        unsafe { slot.value_mut() }
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
        let next_free = slot.claim_next_free(); // 1 stamp read

        inner.free_head.set(next_free);
        inner.len.set(inner.len.get() + 1);

        unsafe { (*slot.value.get()).write(value) };
        slot.set_key_occupied(free_head); // 1 stamp write

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Entry::new(slot_ptr, self.ptr)
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

impl<T> fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .finish()
    }
}

// =============================================================================
// VacantEntry
// =============================================================================

/// A reserved but unfilled slot in the slab.
///
/// Created by [`Slab::try_vacant_entry`]. Fill with [`insert`](Self::insert)
/// or drop to return the slot to the freelist.
#[must_use = "dropping VacantEntry releases the reserved slot"]
pub struct VacantEntry<T> {
    inner: *mut BoundedSlabInner<T>,
    key: Key,
    consumed: bool,
    _marker: PhantomData<T>,
}

impl<T> VacantEntry<T> {
    #[inline]
    fn inner(&self) -> &BoundedSlabInner<T> {
        // SAFETY: inner ptr is valid for 'static
        unsafe { &*self.inner }
    }

    /// Returns the key this slot will have once filled.
    #[inline]
    pub fn key(&self) -> Key {
        self.key
    }

    /// Fills the slot with a value, returning an RAII Entry.
    #[inline]
    pub fn insert(mut self, value: T) -> Entry<T> {
        let key_index = self.key.index();

        let slot_ptr = {
            let slot = self.inner().slot(key_index);
            unsafe {
                (*slot.value.get()).write(value);
            }
            slot.set_key_occupied(key_index); // 1 write (no read needed)
            (slot as *const SlotCell<T>).cast_mut()
        };

        self.consumed = true;

        Entry::new(slot_ptr, self.inner)
    }

    /// Fills the slot using a closure that receives the key.
    ///
    /// Useful for self-referential patterns.
    #[inline]
    pub fn insert_with<F: FnOnce(Key) -> T>(self, f: F) -> Entry<T> {
        let key = self.key;
        self.insert(f(key))
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
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_capacity_basic() {
        let slab: Slab<u64> = Slab::with_capacity(100);
        assert_eq!(slab.capacity(), 100);
        assert_eq!(slab.len(), 0);
        assert!(slab.is_empty());
        assert!(!slab.is_full());
    }

    #[test]
    #[should_panic(expected = "capacity must be non-zero")]
    fn zero_capacity_panics() {
        let _: Slab<u64> = Slab::with_capacity(0);
    }

    #[test]
    fn insert_and_drop() {
        let slab = Slab::with_capacity(16);

        {
            let entry = slab.try_insert(42u64).unwrap();
            assert_eq!(slab.len(), 1);
            assert_eq!(*entry.get(), 42);
        }

        // Entry dropped, slot freed
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn forget_keeps_data() {
        let slab = Slab::with_capacity(16);

        let entry = slab.try_insert(100u64).unwrap();
        let key = entry.forget();

        // Data still exists
        assert_eq!(slab.len(), 1);
        // SAFETY: key is valid (just obtained from forget)
        assert_eq!(unsafe { *slab.get_by_key(key) }, 100);

        // Clean up via remove
        // SAFETY: key is valid
        let value = unsafe { slab.remove_by_key(key) };
        assert_eq!(value, 100);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn entry_from_key() {
        let slab = Slab::with_capacity(16);

        let entry = slab.try_insert(42u64).unwrap();
        let key = entry.forget();

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
        let slab = Slab::with_capacity(2);

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
        let slab = Slab::with_capacity(16);

        let vacant = slab.try_vacant_entry().unwrap();
        let key = vacant.key();
        let entry = vacant.insert(format!("slot-{}", key.index()));

        assert_eq!(*entry.get(), format!("slot-{}", key.index()));
    }

    #[test]
    fn vacant_entry_drop() {
        let slab: Slab<u64> = Slab::with_capacity(16);

        {
            let _vacant = slab.try_vacant_entry().unwrap();
            assert_eq!(slab.len(), 1);
        }

        // Vacant dropped without insert, slot returned
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn insert_with_self_reference() {
        let slab: Slab<(Key, u64)> = Slab::with_capacity(16);

        let entry = slab.try_insert_with(|e| (e.key(), 42u64)).unwrap();

        let (stored_key, value) = *entry.get();
        assert_eq!(stored_key, entry.key());
        assert_eq!(value, 42);
    }

    #[test]
    fn handle_is_copy() {
        let slab = Slab::with_capacity(16);
        let slab2 = slab; // Copy
        let slab3 = slab; // Copy again

        let _e1 = slab.try_insert(1u64).unwrap();
        let _e2 = slab2.try_insert(2u64).unwrap();
        let _e3 = slab3.try_insert(3u64).unwrap();

        assert_eq!(slab.len(), 3);
    }

    #[test]
    fn entry_size() {
        // Entry is 16 bytes: slot ptr (8) + inner ptr (8)
        // Key is stored in slot's stamp, not in Entry
        assert_eq!(std::mem::size_of::<Entry<u64>>(), 16);
    }

    #[test]
    fn take_and_reinsert() {
        let slab = Slab::with_capacity(16);

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
        let slab = Slab::with_capacity(16);
        let mut entry = slab.try_insert(42u64).unwrap();

        let old = entry.replace(100);
        assert_eq!(old, 42);
        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn and_modify() {
        let slab = Slab::with_capacity(16);
        let mut entry = slab.try_insert(0u64).unwrap();

        entry.and_modify(|v| *v += 1).and_modify(|v| *v *= 2);

        assert_eq!(*entry.get(), 2);
    }

    #[test]
    fn explicit_remove() {
        let slab = Slab::with_capacity(16);
        let entry = slab.try_insert(42u64).unwrap();

        let value = entry.remove();
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn clear() {
        let slab = Slab::with_capacity(16);

        // Insert and forget some entries
        for i in 0..5 {
            let entry = slab.try_insert(i as u64).unwrap();
            entry.forget();
        }

        assert_eq!(slab.len(), 5);

        slab.clear();

        assert_eq!(slab.len(), 0);
        assert!(slab.is_empty());
    }
}
