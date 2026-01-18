//! Growable slab with generational keys.
//!
//! [`Slab`] provides O(1) insert, access, and remove with ABA protection.
//! Grows automatically when capacity is exceeded.

use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ops::{Index, IndexMut};

use crate::{BoundedSlab, Key};

const SLAB_NONE: u32 = u32::MAX;
const DEFAULT_BASE_CAPACITY: usize = 4096;

// =============================================================================
// SlabEntry
// =============================================================================

struct SlabEntry<T> {
    inner: BoundedSlab<T>,
    next_slab: u32,
}

// =============================================================================
// Slab
// =============================================================================

/// A growable slab with generational keys.
///
/// `Slab` composes multiple [`BoundedSlab`]s with geometric growth.
/// The first slab has `base_capacity` slots, each subsequent slab doubles.
///
/// # Growth
///
/// Slabs are allocated on demand. Memory is never freed until the `Slab` is dropped.
///
/// # Key Safety
///
/// Keys are generational - stale keys return `None` instead of wrong data.
pub struct Slab<T> {
    slabs: Box<[MaybeUninit<SlabEntry<T>>]>,

    num_slabs: u32,
    slabs_head: u32,

    base_capacity: u32,
    base_shift: u32,

    len: usize,

    _marker: PhantomData<T>,
}

impl<T> Slab<T> {
    // =========================================================================
    // Construction
    // =========================================================================

    /// Creates a new empty slab with default base capacity (4096).
    pub fn new() -> Self {
        Self::with_base_capacity(DEFAULT_BASE_CAPACITY)
    }

    /// Creates a new empty slab with the specified base capacity.
    ///
    /// Base capacity is rounded up to the next power of two.
    /// Each subsequent internal slab doubles in size.
    pub fn with_base_capacity(base_capacity: usize) -> Self {
        let base_capacity = base_capacity.max(1).next_power_of_two().min(1 << 30) as u32;
        let base_shift = base_capacity.trailing_zeros();

        // Max slabs needed to cover u32 index space
        let max_slabs = (32 - base_shift) as usize;

        let slabs = (0..max_slabs)
            .map(|_| MaybeUninit::uninit())
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            slabs,
            num_slabs: 0,
            slabs_head: SLAB_NONE,
            base_capacity,
            base_shift,
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Creates a new slab with pre-allocated capacity.
    ///
    /// Allocates enough slabs to hold at least `capacity` items.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut slab = Self::new();
        while slab.capacity() < capacity {
            slab.grow();
        }
        slab
    }

    // =========================================================================
    // Capacity
    // =========================================================================

    /// Returns the number of occupied slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if no slots are occupied.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the total capacity across all allocated slabs.
    pub fn capacity(&self) -> usize {
        if self.num_slabs == 0 {
            0
        } else {
            (self.base_capacity as usize) * ((1usize << self.num_slabs) - 1)
        }
    }

    // =========================================================================
    // Internal: Geometry
    // =========================================================================

    #[inline]
    fn slab_capacity(&self, slab_idx: u32) -> u32 {
        self.base_capacity << slab_idx
    }

    #[inline]
    fn cumulative_before(&self, slab_idx: u32) -> u32 {
        self.base_capacity
            .wrapping_mul((1u32 << slab_idx).wrapping_sub(1))
    }

    #[inline]
    fn decode(&self, index: u32) -> (u32, u32) {
        let adjusted = (index >> self.base_shift) + 1;
        let slab_idx = 31 - adjusted.leading_zeros();
        let local_index = index - self.cumulative_before(slab_idx);
        (slab_idx, local_index)
    }

    #[inline]
    fn encode(&self, slab_idx: u32, local_index: u32) -> u32 {
        self.cumulative_before(slab_idx) + local_index
    }

    // =========================================================================
    // Internal: Slab Management
    // =========================================================================

    fn grow(&mut self) {
        let slab_idx = self.num_slabs;
        let capacity = self.slab_capacity(slab_idx) as usize;

        let inner = BoundedSlab::with_capacity(capacity);

        let entry = SlabEntry {
            inner,
            next_slab: self.slabs_head,
        };

        self.slabs[slab_idx as usize].write(entry);
        self.slabs_head = slab_idx;
        self.num_slabs += 1;
    }

    #[inline]
    fn entry(&self, slab_idx: u32) -> &SlabEntry<T> {
        debug_assert!(slab_idx < self.num_slabs);
        unsafe { self.slabs[slab_idx as usize].assume_init_ref() }
    }

    #[inline]
    fn entry_mut(&mut self, slab_idx: u32) -> &mut SlabEntry<T> {
        debug_assert!(slab_idx < self.num_slabs);
        unsafe { self.slabs[slab_idx as usize].assume_init_mut() }
    }

    // =========================================================================
    // Insert
    // =========================================================================

    /// Inserts a value, returning its key.
    ///
    /// Grows the slab if necessary.
    pub fn insert(&mut self, value: T) -> Key {
        if self.slabs_head == SLAB_NONE {
            self.grow();
        }
        let slab_idx = self.slabs_head;
        let entry = self.entry_mut(slab_idx);

        // Safety: slabs_head only points to non-full slabs
        let local_key = unsafe { entry.inner.insert_unchecked(value) };

        if entry.inner.is_full() {
            self.slabs_head = entry.next_slab;
        }
        self.len += 1;

        let global_index = self.encode(slab_idx, local_key.index());
        Key::new(global_index)
    }

    /// Returns a vacant entry for deferred insertion.
    ///
    /// Grows the slab if necessary.
    pub fn vacant_entry(&mut self) -> VacantEntry<'_, T> {
        if self.slabs_head == SLAB_NONE {
            self.grow();
        }
        let slab_idx = self.slabs_head;
        let entry = self.entry_mut(slab_idx);

        // Safety: slabs_head only points to non-full slabs
        let local_index = match entry.inner.reserve_slot() {
            Some(idx) => idx,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        VacantEntry {
            slab: self,
            slab_idx,
            local_index,
            inserted: false,
        }
    }

    // =========================================================================
    // Access
    // =========================================================================

    /// Returns a reference to the value for the given key.
    pub fn get(&self, key: Key) -> Option<&T> {
        if key.is_none() {
            return None;
        }
        let (slab_idx, local_index) = self.decode(key.index());
        if slab_idx >= self.num_slabs {
            return None;
        }
        let local_key = Key::new(local_index);
        self.entry(slab_idx).inner.get(local_key)
    }

    /// Returns a mutable reference to the value for the given key.
    pub fn get_mut(&mut self, key: Key) -> Option<&mut T> {
        if key.is_none() {
            return None;
        }
        let (slab_idx, local_index) = self.decode(key.index());
        if slab_idx >= self.num_slabs {
            return None;
        }
        let local_key = Key::new(local_index);
        self.entry_mut(slab_idx).inner.get_mut(local_key)
    }

    /// Returns `true` if the key refers to a valid, occupied slot.
    pub fn contains(&self, key: Key) -> bool {
        if key.is_none() {
            return false;
        }
        let (slab_idx, local_index) = self.decode(key.index());
        if slab_idx >= self.num_slabs {
            return false;
        }
        let local_key = Key::new(local_index);
        self.entry(slab_idx).inner.contains(local_key)
    }

    /// Returns a reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot with matching generation.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        let (slab_idx, local_index) = self.decode(key.index());
        let local_key = Key::new(local_index);
        unsafe { self.entry(slab_idx).inner.get_unchecked(local_key) }
    }

    /// Returns a mutable reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot with matching generation.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: Key) -> &mut T {
        let (slab_idx, local_index) = self.decode(key.index());
        let local_key = Key::new(local_index);
        unsafe { self.entry_mut(slab_idx).inner.get_unchecked_mut(local_key) }
    }

    // =========================================================================
    // Remove
    // =========================================================================

    /// Removes and returns the value for the given key.
    ///
    /// # Panics
    ///
    /// Panics if the key is invalid or stale.
    pub fn remove(&mut self, key: Key) -> T {
        assert!(!key.is_none(), "cannot remove with Key::NONE");

        let (slab_idx, local_index) = self.decode(key.index());
        assert!(slab_idx < self.num_slabs, "key index out of bounds");

        let slabs_head = self.slabs_head;
        let entry = self.entry_mut(slab_idx);
        let was_full = entry.inner.is_full();
        let local_key = Key::new(local_index);
        let value = entry.inner.remove(local_key);

        if was_full {
            entry.next_slab = slabs_head;
            self.slabs_head = slab_idx;
        }
        self.len -= 1;
        value
    }

    /// Removes and returns the value without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot with matching generation.
    pub unsafe fn remove_unchecked(&mut self, key: Key) -> T {
        let (slab_idx, local_index) = self.decode(key.index());

        let slabs_head = self.slabs_head;
        let entry = self.entry_mut(slab_idx);
        let was_full = entry.inner.is_full();

        let local_key = Key::new(local_index);
        let value = unsafe { entry.inner.remove_unchecked(local_key) };

        if was_full {
            entry.next_slab = slabs_head;
            self.slabs_head = slab_idx;
        }
        self.len -= 1;
        value
    }

    // =========================================================================
    // Maintenance
    // =========================================================================

    /// Removes all values from the slab.
    ///
    /// Preserves allocated capacity and generations.
    pub fn clear(&mut self) {
        let num_slabs = self.num_slabs;
        for i in 0..num_slabs {
            let entry = self.entry_mut(i);
            entry.inner.clear();
            entry.next_slab = if i + 1 < num_slabs { i + 1 } else { SLAB_NONE };
        }

        self.slabs_head = if self.num_slabs > 0 { 0 } else { SLAB_NONE };
        self.len = 0;
    }
}

// =============================================================================
// Traits
// =============================================================================

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for Slab<T> {
    fn drop(&mut self) {
        for i in 0..self.num_slabs {
            unsafe {
                self.slabs[i as usize].assume_init_drop();
            }
        }
    }
}

unsafe impl<T: Send> Send for Slab<T> {}

impl<T> Index<Key> for Slab<T> {
    type Output = T;

    fn index(&self, key: Key) -> &Self::Output {
        self.get(key).expect("invalid or stale key")
    }
}

impl<T> IndexMut<Key> for Slab<T> {
    fn index_mut(&mut self, key: Key) -> &mut Self::Output {
        self.get_mut(key).expect("invalid or stale key")
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slab")
            .field("len", &self.len)
            .field("capacity", &self.capacity())
            .field("num_slabs", &self.num_slabs)
            .finish()
    }
}

// =============================================================================
// VacantEntry
// =============================================================================

/// A reserved slot in the slab, ready to be filled.
///
/// Obtained from [`Slab::vacant_entry`]. This reserves a slot and provides
/// the key before the value is inserted, enabling self-referential structures.
///
/// # Example
///
/// ```
/// use nexus_slab::{Slab, Key};
///
/// struct Node {
///     self_key: Key,
///     data: u64,
/// }
///
/// let mut slab = Slab::new();
///
/// let entry = slab.vacant_entry();
/// let key = entry.key();
/// entry.insert(Node { self_key: key, data: 42 });
///
/// assert_eq!(slab.get(key).unwrap().self_key, key);
/// ```
///
/// # Cancellation
///
/// If dropped without calling [`insert`](VacantEntry::insert), the slot
/// is returned to the freelist and the key becomes invalid.
pub struct VacantEntry<'a, T> {
    slab: &'a mut Slab<T>,
    slab_idx: u32,
    local_index: u32,
    inserted: bool,
}

impl<'a, T> VacantEntry<'a, T> {
    /// Returns the key that will be associated with the inserted value.
    #[inline]
    pub fn key(&self) -> Key {
        let global_index = self.slab.encode(self.slab_idx, self.local_index);
        Key::new(global_index)
    }

    /// Inserts a value into the reserved slot, returning the key.
    #[inline]
    pub fn insert(mut self, value: T) -> Key {
        let key = self.key();
        let entry = self.slab.entry_mut(self.slab_idx);
        unsafe {
            entry.inner.fill_reserved(self.local_index, value);
        }
        if entry.inner.is_full() {
            self.slab.slabs_head = entry.next_slab;
        }
        self.slab.len += 1;
        self.inserted = true;
        key
    }
}

impl<T> Drop for VacantEntry<'_, T> {
    fn drop(&mut self) {
        if !self.inserted {
            let entry = self.slab.entry_mut(self.slab_idx);
            unsafe {
                entry.inner.cancel_reserved(self.local_index);
            }
        }
    }
}

impl<T> std::fmt::Debug for VacantEntry<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

    #[test]
    fn new_is_empty() {
        let slab: Slab<u64> = Slab::new();
        assert!(slab.is_empty());
        assert_eq!(slab.len(), 0);
        assert_eq!(slab.capacity(), 0);
    }

    #[test]
    fn insert_get_remove() {
        let mut slab = Slab::new();

        let key = slab.insert(42u64);
        assert_eq!(slab.len(), 1);
        assert_eq!(slab.get(key), Some(&42));

        let removed = slab.remove(key);
        assert_eq!(removed, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn grows_automatically() {
        let mut slab = Slab::with_base_capacity(4);

        let mut keys = Vec::new();
        for i in 0..100u64 {
            keys.push(slab.insert(i));
        }

        assert_eq!(slab.len(), 100);
        assert!(slab.capacity() >= 100);

        for (i, key) in keys.iter().enumerate() {
            assert_eq!(slab.get(*key), Some(&(i as u64)));
        }
    }

    #[test]
    fn vacant_entry_basic() {
        let mut slab = Slab::new();

        let entry = slab.vacant_entry();
        let key = entry.key();
        entry.insert(42u64);

        assert_eq!(slab.get(key), Some(&42));
    }

    #[test]
    fn vacant_entry_self_referential() {
        struct Node {
            self_key: Key,
            data: u64,
        }

        let mut slab = Slab::new();

        let entry = slab.vacant_entry();
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
    fn vacant_entry_drop_cancels() {
        let mut slab = Slab::<usize>::new();

        let key = {
            let entry = slab.vacant_entry();
            entry.key()
        };

        assert_eq!(slab.len(), 0);
        assert_eq!(slab.get(key), None);
    }

    #[test]
    fn clear_preserves_capacity() {
        let mut slab = Slab::new();

        for i in 0..100u64 {
            slab.insert(i);
        }

        let cap_before = slab.capacity();
        slab.clear();

        assert_eq!(slab.len(), 0);
        assert_eq!(slab.capacity(), cap_before);
    }

    #[test]
    fn decode_encode_roundtrip() {
        let slab: Slab<u64> = Slab::with_base_capacity(16);

        for index in [0, 1, 15, 16, 31, 32, 47, 48, 100, 1000] {
            let (slab_idx, local_index) = slab.decode(index);
            let encoded = slab.encode(slab_idx, local_index);
            assert_eq!(encoded, index, "roundtrip failed for {}", index);
        }
    }

    #[test]
    fn geometric_growth() {
        let slab: Slab<u64> = Slab::with_base_capacity(16);

        assert_eq!(slab.slab_capacity(0), 16);
        assert_eq!(slab.slab_capacity(1), 32);
        assert_eq!(slab.slab_capacity(2), 64);
        assert_eq!(slab.slab_capacity(3), 128);
    }

    #[test]
    fn slab_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Slab<u64>>();
        assert_send::<Slab<String>>();
    }

    #[test]
    fn stress_insert_remove() {
        let mut slab = Slab::with_base_capacity(16);
        let mut keys = Vec::new();

        for i in 0..10_000u64 {
            keys.push(slab.insert(i));
        }

        for key in keys.iter() {
            slab.remove(*key);
        }

        assert_eq!(slab.len(), 0);
    }
}
