//! Fixed-capacity slab with generational keys.
//!
//! [`BoundedSlab`] provides O(1) insert, access, and remove with ABA protection
//! via generational keys. Capacity is fixed at construction time.
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
//! assert_eq!(removed, Some("hello"));
//!
//! // Stale key returns None (ABA protected)
//! assert_eq!(slab.get(key), None);
//! ```

use std::marker::PhantomData;
use std::ops::{Index, IndexMut};
use std::ptr::NonNull;
use std::{fmt, ptr};

use crate::sys::Pages;
use crate::{Full, Key, SLOT_NONE, Slot};

// =============================================================================
// BoundedSlab
// =============================================================================

/// A fixed-capacity slab with generational keys.
///
/// `BoundedSlab` allocates all memory upfront and provides O(1) operations
/// with ABA protection through generational keys.
///
/// # Capacity
///
/// Capacity is fixed at construction. Use [`try_insert`](Self::try_insert)
/// which returns `Err(Full)` when the slab is full.
///
/// # Key Safety
///
/// Unlike simple index-based slabs, `BoundedSlab` uses generational keys.
/// When a slot is freed and reused, the generation increments, so stale
/// keys return `None` instead of silently accessing wrong data.
///
/// # Memory Layout
///
/// Slots are stored in a single contiguous allocation. Each slot contains
/// a 64-bit tag (generation + freelist pointer) followed by the value.
#[repr(C)]
pub struct BoundedSlab<T> {
    // Hot - every operation
    ptr: NonNull<Slot<T>>,
    capacity: u32,
    len: u32,
    free_head: u32,

    // Cold
    pages: Pages,
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
        assert!(
            capacity <= SLOT_NONE as usize,
            "capacity exceeds maximum ({})",
            SLOT_NONE
        );

        let slot_size = std::mem::size_of::<Slot<T>>();
        let slot_align = std::mem::align_of::<Slot<T>>();
        let bytes = capacity.checked_mul(slot_size).expect("capacity overflow");

        let pages = Pages::alloc(bytes).expect("allocation failed");

        // Verify alignment
        let raw_ptr = pages.as_ptr();
        assert!(
            raw_ptr as usize % slot_align == 0,
            "Pages allocation not aligned for Slot<T>"
        );

        let ptr = NonNull::new(raw_ptr as *mut Slot<T>).expect("Pages returned null");

        let capacity = capacity as u32;

        // Pre-build freelist: slot[0] -> slot[1] -> ... -> SLOT_NONE
        // This touches all memory, ensuring pages are faulted in.
        unsafe {
            for i in 0..capacity {
                let next = if i + 1 < capacity { i + 1 } else { SLOT_NONE };
                let slot_ptr = ptr.as_ptr().add(i as usize);
                ptr::write(slot_ptr, Slot::new_vacant(next));
            }
        }

        Self {
            pages,
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

    /// Returns the size of the backing allocation in bytes.
    #[inline]
    pub fn memory_size(&self) -> usize {
        self.pages.size()
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

        // Single branch for capacity check
        if free_head == SLOT_NONE {
            return Err(Full(value));
        }

        // Read phase - gather all data we need
        let (next_free, new_gen) = {
            let slot = self.slot(free_head);
            (slot.next_free(), slot.generation().wrapping_add(1))
        };

        // Write phase - all writes batched together
        self.free_head = next_free;
        self.len += 1;

        let slot = self.slot_mut(free_head);
        slot.set_occupied(new_gen);
        slot.value.write(value);

        Ok(Key::new(free_head, new_gen))
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
        let slot = self.slot_mut(index);

        // Compute generation for this allocation
        let old_gen = slot.generation();
        let new_gen = old_gen.wrapping_add(1);

        // Pop from freelist
        self.free_head = slot.next_free();

        Some(VacantEntry {
            slab: self,
            index,
            generation: new_gen,
            inserted: false,
        })
    }

    // =========================================================================
    // Access
    // =========================================================================

    /// Returns a reference to the value for the given key.
    ///
    /// Returns `None` if the key is invalid or stale.
    #[inline(always)]
    pub fn get(&self, key: Key) -> Option<&T> {
        let index = key.index();
        let in_bounds = index < self.capacity;

        // Use index 0 as safe fallback for speculative load
        let safe_index = if in_bounds { index } else { 0 };
        let slot = self.slot(safe_index);

        // Branchless: all conditions computed, single branch at end
        let valid = in_bounds & slot.is_occupied() & (slot.generation() == key.generation());

        if valid {
            Some(unsafe { slot.value.assume_init_ref() })
        } else {
            None
        }
    }

    /// Returns a mutable reference to the value for the given key.
    ///
    /// Returns `None` if the key is invalid or stale.
    #[inline(always)]
    pub fn get_mut(&mut self, key: Key) -> Option<&mut T> {
        let index = key.index();
        let in_bounds = index < self.capacity;

        let safe_index = if in_bounds { index } else { 0 };
        let slot = self.slot(safe_index);

        let valid = in_bounds & slot.is_occupied() & (slot.generation() == key.generation());

        if valid {
            Some(unsafe { self.slot_mut(index).value.assume_init_mut() })
        } else {
            None
        }
    }

    /// Returns `true` if the key refers to a valid, occupied slot.
    #[inline(always)]
    pub fn contains(&self, key: Key) -> bool {
        let index = key.index();
        let in_bounds = index < self.capacity;

        let safe_index = if in_bounds { index } else { 0 };
        let slot = self.slot(safe_index);

        in_bounds & slot.is_occupied() & (slot.generation() == key.generation())
    }

    /// Returns a reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot with matching generation.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        unsafe { self.slot(key.index()).value.assume_init_ref() }
    }

    /// Returns a mutable reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot with matching generation.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: Key) -> &mut T {
        unsafe { self.slot_mut(key.index()).value.assume_init_mut() }
    }

    // =========================================================================
    // Remove
    // =========================================================================

    /// Removes and returns the value for the given key.
    ///
    /// Returns `None` if the key is invalid or stale.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::BoundedSlab;
    ///
    /// let mut slab = BoundedSlab::with_capacity(16);
    /// let key = slab.try_insert(42).unwrap();
    ///
    /// assert_eq!(slab.remove(key), Some(42));
    /// assert_eq!(slab.remove(key), None);  // Already removed
    /// ```
    #[inline(always)]
    pub fn remove(&mut self, key: Key) -> Option<T> {
        let index = key.index();
        let in_bounds = index < self.capacity;

        let safe_index = if in_bounds { index } else { 0 };

        // Read slot state for validation (speculative)
        let (is_occupied, gen_match) = {
            let slot = self.slot(safe_index);
            (slot.is_occupied(), slot.generation() == key.generation())
        };

        let valid = in_bounds & is_occupied & gen_match;

        if !valid {
            return None;
        }

        // Commit phase - we know it's valid now
        let free_head = self.free_head;
        let slot = self.slot_mut(index);
        let value = unsafe { slot.value.assume_init_read() };

        slot.set_vacant(free_head);
        self.free_head = index;
        self.len -= 1;

        Some(value)
    }

    /// Removes and returns the value without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot with matching generation.
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

    /// Locks all pages in physical RAM, preventing swapping.
    ///
    /// See [`sys::Pages::mlock`](crate::sys::Pages::mlock) for details.
    pub fn mlock(&self) -> std::io::Result<()> {
        self.pages.mlock()
    }

    /// Unlocks pages, allowing them to be swapped.
    pub fn munlock(&self) -> std::io::Result<()> {
        self.pages.munlock()
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
        // Pages dropped automatically
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
        self.get(key).expect("invalid or stale key")
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
        self.get_mut(key).expect("invalid or stale key")
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

/// A vacant entry in the slab, ready to be filled.
///
/// Obtained from [`BoundedSlab::try_vacant_entry`]. This reserves a slot
/// and provides the key before the value is inserted.
///
/// If dropped without calling [`insert`](VacantEntry::insert), the slot
/// is returned to the free list and the key becomes invalid.
pub struct VacantEntry<'a, T> {
    slab: &'a mut BoundedSlab<T>,
    index: u32,
    generation: u32,
    inserted: bool,
}

impl<'a, T> VacantEntry<'a, T> {
    /// Returns the key that will be associated with the inserted value.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.index, self.generation)
    }

    /// Inserts a value into the vacant entry, returning the key.
    #[inline]
    pub fn insert(mut self, value: T) -> Key {
        let key = self.key();

        let slot = self.slab.slot_mut(self.index);
        slot.value.write(value);
        slot.set_occupied(self.generation);

        self.slab.len += 1;
        self.inserted = true;

        key
    }
}

impl<T> Drop for VacantEntry<'_, T> {
    fn drop(&mut self) {
        if !self.inserted {
            // Return slot to freelist
            // Note: generation was already incremented when we popped,
            // so we need to preserve it for the next allocation.
            let free_head = self.slab.free_head;
            let slot = self.slab.slot_mut(self.index);

            // Set vacant with the NEW generation (that we computed but didn't use)
            // so the next alloc increments from there
            slot.set_occupied(self.generation); // temporarily set to store generation
            slot.set_vacant(free_head); // now set vacant, preserving generation

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
        assert_eq!(removed, Some(42));
        assert_eq!(slab.len(), 0);
        assert_eq!(slab.get(key), None);
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
    // Generation / Stale Keys
    // =========================================================================

    #[test]
    fn stale_key_returns_none() {
        let mut slab = BoundedSlab::with_capacity(16);

        let key1 = slab.try_insert(1u64).unwrap();
        slab.remove(key1);

        // Reuse same slot
        let key2 = slab.try_insert(2u64).unwrap();

        // Same index, different generation
        assert_eq!(key1.index(), key2.index());
        assert_ne!(key1.generation(), key2.generation());

        // Stale key returns None
        assert_eq!(slab.get(key1), None);
        assert_eq!(slab.get(key2), Some(&2));
    }

    #[test]
    fn stale_key_remove_returns_none() {
        let mut slab = BoundedSlab::with_capacity(16);

        let key1 = slab.try_insert(1u64).unwrap();
        slab.remove(key1);
        let _key2 = slab.try_insert(2u64).unwrap();

        // Can't remove with stale key
        assert_eq!(slab.remove(key1), None);
    }

    #[test]
    fn generation_increments_on_reuse() {
        let mut slab = BoundedSlab::with_capacity(1);

        let mut last_gen = 0;
        for i in 0..100u64 {
            let key = slab.try_insert(i).unwrap();
            assert!(key.generation() > last_gen || (last_gen == 0 && key.generation() == 1));
            last_gen = key.generation();
            slab.remove(key);
        }
    }

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
    fn clear_preserves_generations() {
        let mut slab = BoundedSlab::with_capacity(1);

        let key1 = slab.try_insert(1u64).unwrap();
        let gen1 = key1.generation();

        slab.clear();

        let key2 = slab.try_insert(2u64).unwrap();
        let gen2 = key2.generation();

        // Generation should have incremented
        assert!(gen2 > gen1);
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
    #[should_panic(expected = "invalid or stale key")]
    fn index_stale_key_panics() {
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
            assert_eq!(slab.remove(key), Some(i));
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
