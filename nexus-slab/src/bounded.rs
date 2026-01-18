//! Fixed-capacity slab allocator.
//!
//! [`BoundedSlab`] provides O(1) insert, access, and remove with fixed
//! capacity determined at construction time.
//!
//! # Example
//!
//! ```
//! use nexus_slab::BoundedSlab;
//!
//! let mut slab = BoundedSlab::with_capacity(1024);
//!
//! let key = slab.try_insert("hello").unwrap();
//! assert_eq!(slab.get(key), Some(&"hello"));
//!
//! let removed = slab.remove(key);
//! assert_eq!(removed, "hello");
//!
//! // Key returns None after removal
//! assert_eq!(slab.get(key), None);
//! ```

use std::alloc::{Layout, alloc, dealloc};
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};
use std::ptr::NonNull;
use std::{fmt, ptr};

use crate::{Full, Key, SLOT_NONE, Slot};

// =============================================================================
// BoundedSlab
// =============================================================================

/// A fixed-capacity slab allocator.
///
/// `BoundedSlab` allocates all memory upfront and provides O(1) operations.
///
/// # Capacity
///
/// Capacity is fixed at construction. Use [`try_insert`](Self::try_insert)
/// which returns `Err(Full)` when the slab is full.
///
/// # Key Validity
///
/// Keys are simple indices. After a slot is removed, its key becomes invalid
/// and `get()`/`get_mut()` will return `None`. The slab checks occupancy
/// but does not track key reuse—if you insert a new value and it occupies
/// the same slot, an old key will access the new value.
///
/// For systems requiring protection against stale key reuse, validate against
/// authoritative external identifiers (see [`Key`](crate::Key) documentation).
///
/// # Memory Layout
///
/// Slots are stored in a single contiguous allocation. Each slot contains
/// a 32-bit tag followed by the value.
#[repr(C)]
pub struct BoundedSlab<T> {
    ptr: NonNull<Slot<T>>,
    capacity: u32,
    len: u32,
    free_head: u32,
    _marker: PhantomData<T>,
}

impl<T> BoundedSlab<T> {
    // =========================================================================
    // Construction
    // =========================================================================

    /// Creates a new slab with the given capacity.
    ///
    /// All slots are pre-touched during construction, so no page faults
    /// occur during normal operation.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `capacity` is zero
    /// - `capacity` exceeds maximum (~2 billion)
    /// - Memory allocation fails
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let slab: BoundedSlab<u64> = BoundedSlab::with_capacity(1000);
    /// assert_eq!(slab.capacity(), 1000);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be non-zero");
        assert!(capacity <= SLOT_NONE as usize, "capacity exceeds maximum");

        let layout = Layout::array::<Slot<T>>(capacity).expect("capacity overflow");

        let ptr = unsafe { alloc(layout) } as *mut Slot<T>;
        let ptr = NonNull::new(ptr).expect("allocation failed");

        let capacity = capacity as u32;

        // Pre-touch: build freelist, fault in pages
        unsafe {
            for i in 0..capacity {
                let next = if i + 1 < capacity { i + 1 } else { SLOT_NONE };
                ptr.as_ptr().add(i as usize).write(Slot::new_vacant(next));
            }
        }

        Self {
            ptr,
            capacity,
            len: 0,
            free_head: 0,
            _marker: PhantomData,
        }
    }

    // =========================================================================
    // Capacity
    // =========================================================================

    /// Returns the total number of slots.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity as usize
    }

    /// Returns the number of occupied slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Returns `true` if no slots are occupied.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns `true` if all slots are occupied.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.free_head == SLOT_NONE
    }

    // =========================================================================
    // Internal Helpers
    // =========================================================================

    #[inline]
    fn slot(&self, index: u32) -> &Slot<T> {
        debug_assert!(index < self.capacity);
        unsafe { &*self.ptr.as_ptr().add(index as usize) }
    }

    #[inline]
    fn slot_mut(&mut self, index: u32) -> &mut Slot<T> {
        debug_assert!(index < self.capacity);
        unsafe { &mut *self.ptr.as_ptr().add(index as usize) }
    }

    /// Reserves a slot without inserting. Returns the index.
    ///
    /// # Safety
    ///
    /// Caller must either call `fill_reserved` or `cancel_reserved` with
    /// the returned index before any other operations on this slab.
    #[inline(always)]
    pub(crate) fn reserve_slot(&mut self) -> Option<u32> {
        if self.is_full() {
            return None;
        }
        let index = self.free_head;
        let next_free = self.slot(index).next_free();
        self.free_head = next_free;
        Some(index)
    }

    /// Fills a reserved slot with a value.
    ///
    /// # Safety
    ///
    /// `index` must have been returned by `reserve_slot` and not yet
    /// filled or cancelled.
    #[inline(always)]
    pub(crate) unsafe fn fill_reserved(&mut self, index: u32, value: T) {
        let slot = self.slot_mut(index);
        slot.value.write(value);
        slot.set_occupied();
        self.len += 1;
    }

    /// Cancels a reservation, returning the slot to the freelist.
    ///
    /// # Safety
    ///
    /// `index` must have been returned by `reserve_slot` and not yet
    /// filled or cancelled.
    #[inline(always)]
    pub(crate) unsafe fn cancel_reserved(&mut self, index: u32) {
        let free_head = self.free_head;
        let slot = self.slot_mut(index);
        slot.set_vacant(free_head);
        self.free_head = index;
    }

    // =========================================================================
    // Insert
    // =========================================================================

    /// Inserts a value, returning its key.
    ///
    /// Returns `Err(Full(value))` if the slab is full, allowing recovery
    /// of the value.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let mut slab = BoundedSlab::with_capacity(2);
    ///
    /// let k1 = slab.try_insert("a").unwrap();
    /// let k2 = slab.try_insert("b").unwrap();
    ///
    /// // Slab is full
    /// let err = slab.try_insert("c").unwrap_err();
    /// assert_eq!(err.0, "c");
    /// ```
    pub fn try_insert(&mut self, value: T) -> Result<Key, Full<T>> {
        let free_head = self.free_head;
        if free_head == SLOT_NONE {
            return Err(Full(value));
        }
        let next_free = self.slot(free_head).next_free();
        self.free_head = next_free;
        self.len += 1;
        let slot = self.slot_mut(free_head);
        slot.set_occupied();
        slot.value.write(value);
        Ok(Key::new(free_head))
    }

    /// Inserts a value without checking capacity.
    ///
    /// # Safety
    ///
    /// Caller must ensure the slab is not full (`!is_full()`).
    #[inline(always)]
    pub(crate) unsafe fn insert_unchecked(&mut self, value: T) -> (u32, bool) {
        let free_head = self.free_head;
        debug_assert!(free_head != SLOT_NONE, "insert_unchecked on full slab");

        let next_free = self.slot(free_head).next_free();
        self.free_head = next_free;
        self.len += 1;

        let slot = self.slot_mut(free_head);
        slot.set_occupied();
        slot.value.write(value);

        (free_head, next_free == SLOT_NONE)
    }

    /// Returns a vacant entry for deferred insertion.
    ///
    /// This reserves a slot and provides the key before the value is inserted,
    /// enabling self-referential structures.
    ///
    /// Returns `None` if the slab is full.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::{BoundedSlab, Key};
    ///
    /// struct Node {
    ///     self_key: Key,
    ///     data: u64,
    /// }
    ///
    /// let mut slab = BoundedSlab::with_capacity(16);
    ///
    /// let entry = slab.try_vacant_entry().unwrap();
    /// let key = entry.key();
    /// entry.insert(Node { self_key: key, data: 42 });
    ///
    /// assert_eq!(slab.get(key).unwrap().self_key, key);
    /// ```
    pub fn try_vacant_entry(&mut self) -> Option<VacantEntry<'_, T>> {
        if self.is_full() {
            return None;
        }
        let index = self.free_head;
        let next_free = self.slot(index).next_free();
        self.free_head = next_free;
        Some(VacantEntry {
            slab: self,
            index,
            inserted: false,
        })
    }

    // =========================================================================
    // Access
    // =========================================================================

    /// Returns a reference to the value for the given key.
    ///
    /// Returns `None` if the key is out of bounds or the slot is vacant.
    pub fn get(&self, key: Key) -> Option<&T> {
        let index = key.index();
        if index >= self.capacity {
            return None;
        }
        let slot = self.slot(index);
        if slot.is_occupied() {
            Some(unsafe { slot.value.assume_init_ref() })
        } else {
            None
        }
    }

    /// Returns a mutable reference to the value for the given key.
    ///
    /// Returns `None` if the key is out of bounds or the slot is vacant.
    pub fn get_mut(&mut self, key: Key) -> Option<&mut T> {
        let index = key.index();
        if index >= self.capacity {
            return None;
        }
        let slot = self.slot_mut(index);
        if slot.is_occupied() {
            Some(unsafe { slot.value.assume_init_mut() })
        } else {
            None
        }
    }

    /// Returns `true` if the key refers to a valid, occupied slot.
    pub fn contains(&self, key: Key) -> bool {
        let index = key.index();
        if index >= self.capacity {
            return false;
        }
        self.slot(index).is_occupied()
    }

    /// Returns a reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        unsafe { self.slot(key.index()).value.assume_init_ref() }
    }

    /// Returns a mutable reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: Key) -> &mut T {
        unsafe { self.slot_mut(key.index()).value.assume_init_mut() }
    }

    /// Returns a reference if occupied, without bounds checking.
    ///
    /// # Safety
    ///
    /// `key.index()` must be less than `capacity`.
    #[inline]
    pub(crate) unsafe fn get_occupied_unchecked(&self, key: Key) -> Option<&T> {
        let slot = self.slot(key.index());
        if slot.is_occupied() {
            Some(unsafe { slot.value.assume_init_ref() })
        } else {
            None
        }
    }

    /// Returns a mutable reference if occupied, without bounds checking.
    ///
    /// # Safety
    ///
    /// `key.index()` must be less than `capacity`.
    #[inline]
    pub(crate) unsafe fn get_mut_occupied_unchecked(&mut self, key: Key) -> Option<&mut T> {
        let slot = self.slot_mut(key.index());
        if slot.is_occupied() {
            Some(unsafe { slot.value.assume_init_mut() })
        } else {
            None
        }
    }

    // =========================================================================
    // Remove
    // =========================================================================

    /// Removes and returns the value for the given key.
    ///
    /// # Panics
    ///
    /// Panics if the key is invalid or stale.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let mut slab = BoundedSlab::with_capacity(16);
    /// let key = slab.try_insert(42).unwrap();
    ///
    /// assert_eq!(slab.remove(key), 42);
    /// // slab.remove(key);  // Would panic - already removed
    /// ```
    pub fn remove(&mut self, key: Key) -> T {
        let index = key.index();
        assert!(index < self.capacity, "key index out of bounds");

        let free_head = self.free_head;
        let slot = self.slot_mut(index);
        assert!(slot.is_occupied(), "slot is vacant");

        let value = unsafe { slot.value.assume_init_read() };
        slot.set_vacant(free_head);
        self.free_head = index;
        self.len -= 1;
        value
    }

    /// Returns a mutable reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot.
    pub unsafe fn remove_unchecked(&mut self, key: Key) -> T {
        let index = key.index();
        let free_head = self.free_head;
        let slot = self.slot_mut(index);

        let value = unsafe { slot.value.assume_init_read() };

        slot.set_vacant(free_head);
        self.free_head = index;
        self.len -= 1;

        value
    }

    // =========================================================================
    // Maintenance
    // =========================================================================

    /// Removes all values from the slab.
    ///
    /// This drops all contained values and rebuilds the freelist.
    /// Generations are preserved, so stale keys remain invalid.
    ///
    /// More efficient than removing items one by one.
    pub fn clear(&mut self) {
        if self.len == 0 {
            return;
        }

        // Drop all occupied values, rebuild freelist, preserve generations
        let capacity = self.capacity;
        for i in 0..capacity {
            let slot = self.slot_mut(i);
            if slot.is_occupied() {
                unsafe { ptr::drop_in_place(slot.value.as_mut_ptr()) };
            }
            let next = if i + 1 < capacity { i + 1 } else { SLOT_NONE };
            slot.set_vacant(next);
        }

        self.len = 0;
        self.free_head = 0;
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl<T> Drop for BoundedSlab<T> {
    fn drop(&mut self) {
        // Drop all occupied values
        for i in 0..self.capacity {
            if self.slot(i).is_occupied() {
                unsafe { ptr::drop_in_place(self.slot_mut(i).value.as_mut_ptr()) };
            }
        }

        // Recompute layout for dealloc
        let layout = Layout::array::<Slot<T>>(self.capacity as usize)
            .expect("layout was valid at construction");
        unsafe { dealloc(self.ptr.as_ptr() as *mut u8, layout) };
    }
}

// Safety: BoundedSlab owns its data and can be sent if T can be.
unsafe impl<T: Send> Send for BoundedSlab<T> {}

impl<T> Index<Key> for BoundedSlab<T> {
    type Output = T;

    /// Returns a reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if the key is invalid or stale.
    #[inline]
    fn index(&self, key: Key) -> &Self::Output {
        self.get(key).expect("invalid key")
    }
}

impl<T> IndexMut<Key> for BoundedSlab<T> {
    /// Returns a mutable reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if the key is invalid or stale.
    #[inline]
    fn index_mut(&mut self, key: Key) -> &mut Self::Output {
        self.get_mut(key).expect("invalid key")
    }
}

impl<T: fmt::Debug> fmt::Debug for BoundedSlab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundedSlab")
            .field("len", &self.len)
            .field("capacity", &self.capacity)
            .finish()
    }
}

// =============================================================================
// VacantEntry
// =============================================================================

/// A reserved slot in the slab, ready to be filled.
///
/// Obtained from [`BoundedSlab::try_vacant_entry`]. This reserves a slot
/// and provides the key before the value is inserted, enabling
/// self-referential structures where the value needs to know its own key.
///
/// # Self-Referential Example
///
/// ```
/// use nexus_slab::{BoundedSlab, Key};
///
/// struct Node {
///     self_key: Key,
///     data: u64,
/// }
///
/// let mut slab = BoundedSlab::with_capacity(16);
///
/// let entry = slab.try_vacant_entry().unwrap();
/// let key = entry.key();
/// entry.insert(Node { self_key: key, data: 42 });
///
/// assert_eq!(slab.get(key).unwrap().self_key, key);
/// ```
///
/// # Cancellation
///
/// If dropped without calling [`insert`](VacantEntry::insert), the slot
/// is returned to the freelist and the key becomes invalid. This is safe
/// and does not leak memory.
///
/// ```
/// use nexus_slab::BoundedSlab;
///
/// let mut slab = BoundedSlab::<u64>::with_capacity(16);
///
/// let key = {
///     let entry = slab.try_vacant_entry().unwrap();
///     entry.key()
///     // Dropped without insert
/// };
///
/// assert!(slab.is_empty());
/// assert!(slab.get(key).is_none());
/// ```
pub struct VacantEntry<'a, T> {
    slab: &'a mut BoundedSlab<T>,
    index: u32,
    inserted: bool,
}

impl<'a, T> VacantEntry<'a, T> {
    /// Returns the key that will be associated with the inserted value.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.index)
    }

    /// Inserts a value into the reserved slot, returning the key.
    #[inline]
    pub fn insert(mut self, value: T) -> Key {
        let key = self.key();
        let slot = self.slab.slot_mut(self.index);
        slot.value.write(value);
        slot.set_occupied();
        self.slab.len += 1;
        self.inserted = true;
        key
    }
}

impl<T> Drop for VacantEntry<'_, T> {
    fn drop(&mut self) {
        if !self.inserted {
            let free_head = self.slab.free_head;
            let slot = self.slab.slot_mut(self.index);
            slot.set_vacant(free_head);
            self.slab.free_head = self.index;
        }
    }
}

impl<T> fmt::Debug for VacantEntry<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VacantEntry")
            .field("key", &self.key())
            .finish()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Construction
    // =========================================================================

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

    // =========================================================================
    // Insert / Get / Remove
    // =========================================================================

    #[test]
    fn insert_get_remove() {
        let mut slab = BoundedSlab::with_capacity(16);

        let key = slab.try_insert(42u64).unwrap();
        assert_eq!(slab.len(), 1);
        assert_eq!(slab.get(key), Some(&42));

        let removed = slab.remove(key);
        assert_eq!(removed, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn insert_full_returns_error() {
        let mut slab = BoundedSlab::with_capacity(2);

        slab.try_insert(1u64).unwrap();
        slab.try_insert(2u64).unwrap();

        let err = slab.try_insert(3u64).unwrap_err();
        assert_eq!(err.0, 3);
    }

    #[test]
    fn get_mut_modifies_value() {
        let mut slab = BoundedSlab::with_capacity(16);

        let key = slab.try_insert(10u64).unwrap();
        *slab.get_mut(key).unwrap() = 20;
        assert_eq!(slab.get(key), Some(&20));
    }

    #[test]
    fn contains() {
        let mut slab = BoundedSlab::with_capacity(16);

        let key = slab.try_insert(42u64).unwrap();
        assert!(slab.contains(key));

        slab.remove(key);
        assert!(!slab.contains(key));
    }

    // =========================================================================
    // Invalid Keys
    // =========================================================================

    #[test]
    fn key_none_returns_none() {
        let slab: BoundedSlab<u64> = BoundedSlab::with_capacity(16);
        assert_eq!(slab.get(Key::NONE), None);
    }

    // =========================================================================
    // VacantEntry
    // =========================================================================

    #[test]
    fn vacant_entry_basic() {
        let mut slab = BoundedSlab::with_capacity(16);

        let entry = slab.try_vacant_entry().unwrap();
        let key = entry.key();
        let returned_key = entry.insert(42u64);

        assert_eq!(key, returned_key);
        assert_eq!(slab.get(key), Some(&42));
    }

    #[test]
    fn vacant_entry_self_referential() {
        #[derive(Debug, PartialEq)]
        struct Node {
            self_key: Key,
            data: u64,
        }

        let mut slab = BoundedSlab::with_capacity(16);

        let entry = slab.try_vacant_entry().unwrap();
        let key = entry.key();
        entry.insert(Node {
            self_key: key,
            data: 42,
        });

        let node = slab.get(key).unwrap();
        assert_eq!(node.self_key, key);
        assert_eq!(node.data, 42);
    }

    #[test]
    fn vacant_entry_drop_returns_slot() {
        let mut slab = BoundedSlab::<usize>::with_capacity(16);

        let key = {
            let entry = slab.try_vacant_entry().unwrap();
            entry.key()
            // Dropped without insert
        };

        assert_eq!(slab.len(), 0);
        assert!(!slab.is_full());

        // Key should be invalid (slot returned to freelist, generation consumed)
        assert_eq!(slab.get(key), None);
    }

    #[test]
    fn vacant_entry_full_returns_none() {
        let mut slab = BoundedSlab::with_capacity(1);
        slab.try_insert(1u64).unwrap();

        assert!(slab.try_vacant_entry().is_none());
    }

    // =========================================================================
    // Clear
    // =========================================================================

    #[test]
    fn clear_empties_slab() {
        let mut slab = BoundedSlab::with_capacity(16);

        let k1 = slab.try_insert(1u64).unwrap();
        let k2 = slab.try_insert(2u64).unwrap();
        let k3 = slab.try_insert(3u64).unwrap();

        slab.clear();

        assert_eq!(slab.len(), 0);
        assert!(slab.is_empty());
        assert_eq!(slab.get(k1), None);
        assert_eq!(slab.get(k2), None);
        assert_eq!(slab.get(k3), None);
    }

    #[test]
    fn clear_allows_reuse() {
        let mut slab = BoundedSlab::with_capacity(16);

        for i in 0..10u64 {
            slab.try_insert(i).unwrap();
        }
        slab.clear();

        for i in 0..10u64 {
            slab.try_insert(i * 100).unwrap();
        }
        assert_eq!(slab.len(), 10);
    }

    #[test]
    fn clear_drops_values() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug)]
        struct DropCounter;
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        DROP_COUNT.store(0, Ordering::SeqCst);

        let mut slab = BoundedSlab::with_capacity(16);
        slab.try_insert(DropCounter).unwrap();
        slab.try_insert(DropCounter).unwrap();
        slab.try_insert(DropCounter).unwrap();

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);

        slab.clear();

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn clear_on_empty_is_noop() {
        let mut slab: BoundedSlab<u64> = BoundedSlab::with_capacity(16);
        slab.clear();
        assert_eq!(slab.len(), 0);
    }

    // =========================================================================
    // Drop
    // =========================================================================

    #[test]
    fn drop_cleans_up_values() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug)]
        struct DropCounter;
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        DROP_COUNT.store(0, Ordering::SeqCst);

        {
            let mut slab = BoundedSlab::with_capacity(16);
            slab.try_insert(DropCounter).unwrap();
            slab.try_insert(DropCounter).unwrap();
            slab.try_insert(DropCounter).unwrap();
        }

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn remove_drops_value() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug)]
        struct DropCounter;
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        DROP_COUNT.store(0, Ordering::SeqCst);

        let mut slab = BoundedSlab::with_capacity(16);
        let key = slab.try_insert(DropCounter).unwrap();

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);

        slab.remove(key);

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    }

    // =========================================================================
    // Index Traits
    // =========================================================================

    #[test]
    fn index_trait() {
        let mut slab = BoundedSlab::with_capacity(16);
        let key = slab.try_insert(42u64).unwrap();

        assert_eq!(slab[key], 42);
    }

    #[test]
    fn index_mut_trait() {
        let mut slab = BoundedSlab::with_capacity(16);
        let key = slab.try_insert(42u64).unwrap();

        slab[key] = 100;
        assert_eq!(slab[key], 100);
    }

    #[test]
    #[should_panic(expected = "invalid key")]
    fn index_removed_key_panics() {
        let mut slab = BoundedSlab::with_capacity(16);
        let key = slab.try_insert(42u64).unwrap();
        slab.remove(key);

        let _ = slab[key];
    }

    // =========================================================================
    // Send
    // =========================================================================

    #[test]
    fn slab_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<BoundedSlab<u64>>();
        assert_send::<BoundedSlab<String>>();
    }

    #[test]
    fn move_across_threads() {
        use std::thread;

        let mut slab = BoundedSlab::with_capacity(100);
        let key = slab.try_insert(42u64).unwrap();

        let handle = thread::spawn(move || {
            assert_eq!(slab.get(key), Some(&42));
            slab.remove(key);
            slab
        });

        let slab = handle.join().unwrap();
        assert_eq!(slab.len(), 0);
    }

    // =========================================================================
    // Stress Tests
    // =========================================================================

    #[test]
    fn stress_insert_remove_cycle() {
        let mut slab = BoundedSlab::with_capacity(1);

        for i in 0..10_000u64 {
            let key = slab.try_insert(i).unwrap();
            assert_eq!(slab.get(key), Some(&i));
            assert_eq!(slab.remove(key), i);
        }
    }

    #[test]
    fn stress_fill_drain_cycles() {
        let mut slab = BoundedSlab::with_capacity(64);
        let capacity = slab.capacity();

        for _ in 0..100 {
            let mut keys = Vec::with_capacity(capacity);

            for i in 0..capacity {
                keys.push(slab.try_insert(i as u64).unwrap());
            }
            assert!(slab.is_full());

            for key in keys {
                slab.remove(key);
            }
            assert!(slab.is_empty());
        }
    }
}
