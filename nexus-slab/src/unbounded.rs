//! Growable slab with generational keys.
//!
//! [`Slab`] provides O(1) insert, access, and remove with ABA protection.
//! Grows automatically when capacity is exceeded.

use std::ops::{Index, IndexMut};

use crate::{BoundedSlab, Key};

const SLAB_NONE: u32 = u32::MAX;

// =============================================================================
// SlabEntry
// =============================================================================

/// Internal wrapper pairing a bounded slab with freelist linkage.
struct SlabEntry<T> {
    inner: BoundedSlab<T>,
    next_with_space: u32,
}

// =============================================================================
// Slab
// =============================================================================

/// A growable slab allocator with O(1) operations.
///
/// `Slab` composes multiple fixed-size [`BoundedSlab`]s. Each chunk has the
/// same capacity, enabling fast index decoding via shift and mask.
///
/// # Growth
///
/// When full, a new chunk is allocated on demand. Each growth allocates
/// exactly one chunk—no geometric doubling. Memory is never freed until
/// the `Slab` is dropped.
///
/// # Key Encoding
///
/// Keys encode both chunk index and local slot index:
///
/// ```text
/// ┌─────────────────────┬──────────────────────┐
/// │  chunk_idx (high)   │  local_idx (low)     │
/// └─────────────────────┴──────────────────────┘
/// ```
///
/// Decoding is two instructions: shift and mask.
///
/// # Memory Layout
///
/// Chunks are independent allocations. No copying occurs during growth.
pub struct Slab<T> {
    slabs: Vec<SlabEntry<T>>,
    head_with_space: u32,

    chunk_capacity: u32,
    chunk_shift: u32,
    chunk_mask: u32,

    len: usize,
}

impl<T> Slab<T> {
    // =========================================================================
    // Construction
    // =========================================================================

    /// Creates a new empty slab with the specified chunk capacity.
    ///
    /// Chunk capacity is rounded up to the next power of two.
    /// All chunks will have this same capacity.
    ///
    /// No memory is allocated until the first insert.
    ///
    /// # Panics
    ///
    /// Panics if `chunk_capacity` is zero or exceeds `2^30`.
    pub fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        assert!(chunk_capacity > 0, "chunk_capacity must be non-zero");
        assert!(chunk_capacity <= 1 << 30, "chunk_capacity exceeds maximum");

        let chunk_capacity = chunk_capacity.next_power_of_two() as u32;
        let chunk_shift = chunk_capacity.trailing_zeros();
        let chunk_mask = chunk_capacity - 1;

        Self {
            slabs: Vec::new(),
            head_with_space: SLAB_NONE,
            chunk_capacity,
            chunk_shift,
            chunk_mask,
            len: 0,
        }
    }

    /// Creates a new slab with pre-allocated capacity.
    ///
    /// Allocates enough chunks to hold at least `capacity` items.
    pub fn with_capacity(chunk_capacity: usize, total_capacity: usize) -> Self {
        let mut slab = Self::with_chunk_capacity(chunk_capacity);
        while slab.capacity() < total_capacity {
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

    /// Returns the total capacity across all allocated chunks.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slabs.len() * self.chunk_capacity as usize
    }

    /// Returns the number of allocated chunks.
    #[inline]
    pub fn num_chunks(&self) -> usize {
        self.slabs.len()
    }

    // =========================================================================
    // Internal: Geometry
    // =========================================================================

    /// Decodes a global index into (chunk_idx, local_idx).
    #[inline]
    fn decode(&self, index: u32) -> (u32, u32) {
        let chunk_idx = index >> self.chunk_shift;
        let local_idx = index & self.chunk_mask;
        (chunk_idx, local_idx)
    }

    /// Encodes (chunk_idx, local_idx) into a global index.
    #[inline]
    fn encode(&self, chunk_idx: u32, local_idx: u32) -> u32 {
        (chunk_idx << self.chunk_shift) | local_idx
    }

    // =========================================================================
    // Internal: Slab Management
    // =========================================================================

    /// Allocates a new chunk and adds it to the freelist.
    fn grow(&mut self) {
        let chunk_idx = self.slabs.len() as u32;
        let inner = BoundedSlab::with_capacity(self.chunk_capacity as usize);

        let entry = SlabEntry {
            inner,
            next_with_space: self.head_with_space,
        };

        self.slabs.push(entry);
        self.head_with_space = chunk_idx;
    }

    #[inline]
    fn entry(&self, chunk_idx: u32) -> &SlabEntry<T> {
        debug_assert!((chunk_idx as usize) < self.slabs.len());
        unsafe { self.slabs.get_unchecked(chunk_idx as usize) }
    }

    #[inline]
    fn entry_mut(&mut self, chunk_idx: u32) -> &mut SlabEntry<T> {
        debug_assert!((chunk_idx as usize) < self.slabs.len());
        unsafe { self.slabs.get_unchecked_mut(chunk_idx as usize) }
    }

    // =========================================================================
    // Insert
    // =========================================================================

    /// Inserts a value, returning its key.
    ///
    /// Grows the slab if necessary.
    pub fn insert(&mut self, value: T) -> Key {
        if self.head_with_space == SLAB_NONE {
            self.grow();
        }

        let chunk_idx = self.head_with_space;
        let entry = self.entry_mut(chunk_idx);

        // Safety: head_with_space only points to non-full slabs
        let (local_idx, became_full) = unsafe { entry.inner.insert_unchecked(value) };

        if became_full {
            self.head_with_space = entry.next_with_space;
        }

        self.len += 1;
        Key::new(self.encode(chunk_idx, local_idx))
    }

    /// Returns a vacant entry for deferred insertion.
    ///
    /// Grows the slab if necessary.
    pub fn vacant_entry(&mut self) -> VacantEntry<'_, T> {
        if self.head_with_space == SLAB_NONE {
            self.grow();
        }

        let chunk_idx = self.head_with_space;
        let entry = self.entry_mut(chunk_idx);

        // Safety: head_with_space only points to non-full slabs
        let local_idx = match entry.inner.reserve_slot() {
            Some(idx) => idx,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        VacantEntry {
            slab: self,
            chunk_idx,
            local_idx,
            inserted: false,
        }
    }

    // =========================================================================
    // Access
    // =========================================================================

    /// Returns a reference to the value for the given key.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    pub fn get(&self, key: Key) -> Option<&T> {
        if key.is_none() {
            return None;
        }
        let (chunk_idx, local_idx) = self.decode(key.index());
        if chunk_idx >= self.slabs.len() as u32 {
            return None;
        }
        let local_key = Key::new(local_idx);
        // Safety: decode guarantees local_idx < chunk_capacity
        unsafe {
            self.entry(chunk_idx)
                .inner
                .get_occupied_unchecked(local_key)
        }
    }

    /// Returns a mutable reference to the value for the given key.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    pub fn get_mut(&mut self, key: Key) -> Option<&mut T> {
        if key.is_none() {
            return None;
        }
        let (chunk_idx, local_idx) = self.decode(key.index());
        if chunk_idx >= self.slabs.len() as u32 {
            return None;
        }
        let local_key = Key::new(local_idx);
        // Safety: decode guarantees local_idx < chunk_capacity
        unsafe {
            self.entry_mut(chunk_idx)
                .inner
                .get_mut_occupied_unchecked(local_key)
        }
    }

    /// Returns `true` if the key refers to a valid, occupied slot.
    pub fn contains(&self, key: Key) -> bool {
        if key.is_none() {
            return false;
        }
        let (chunk_idx, local_idx) = self.decode(key.index());
        if chunk_idx >= self.slabs.len() as u32 {
            return false;
        }
        let local_key = Key::new(local_idx);
        // Safety: decode guarantees local_idx < chunk_capacity
        unsafe {
            self.entry(chunk_idx)
                .inner
                .get_occupied_unchecked(local_key)
                .is_some()
        }
    }

    /// Returns a reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        let (chunk_idx, local_idx) = self.decode(key.index());
        let local_key = Key::new(local_idx);
        unsafe { self.entry(chunk_idx).inner.get_unchecked(local_key) }
    }

    /// Returns a mutable reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: Key) -> &mut T {
        let (chunk_idx, local_idx) = self.decode(key.index());
        let local_key = Key::new(local_idx);
        unsafe { self.entry_mut(chunk_idx).inner.get_unchecked_mut(local_key) }
    }

    // =========================================================================
    // Remove
    // =========================================================================

    /// Removes and returns the value for the given key.
    ///
    /// # Panics
    ///
    /// Panics if the key is invalid or the slot is vacant.
    pub fn remove(&mut self, key: Key) -> T {
        assert!(!key.is_none(), "cannot remove with Key::NONE");

        let (chunk_idx, local_idx) = self.decode(key.index());
        assert!(
            (chunk_idx as usize) < self.slabs.len(),
            "key index out of bounds"
        );

        let head_with_space = self.head_with_space;
        let entry = self.entry_mut(chunk_idx);
        let was_full = entry.inner.is_full();

        let local_key = Key::new(local_idx);
        let value = entry.inner.remove(local_key);

        if was_full {
            entry.next_with_space = head_with_space;
            self.head_with_space = chunk_idx;
        }

        self.len -= 1;
        value
    }

    /// Removes and returns the value without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot.
    pub unsafe fn remove_unchecked(&mut self, key: Key) -> T {
        let (chunk_idx, local_idx) = self.decode(key.index());

        let head_with_space = self.head_with_space;
        let entry = self.entry_mut(chunk_idx);
        let was_full = entry.inner.is_full();

        let local_key = Key::new(local_idx);
        let value = unsafe { entry.inner.remove_unchecked(local_key) };

        if was_full {
            entry.next_with_space = head_with_space;
            self.head_with_space = chunk_idx;
        }

        self.len -= 1;
        value
    }

    // =========================================================================
    // Maintenance
    // =========================================================================

    /// Removes all values from the slab.
    ///
    /// Preserves allocated capacity. All chunks are cleared and
    /// added back to the freelist.
    pub fn clear(&mut self) {
        if self.len == 0 {
            return;
        }

        let num_chunks = self.slabs.len() as u32;
        for i in 0..num_chunks {
            let entry = self.entry_mut(i);
            entry.inner.clear();
            entry.next_with_space = if i + 1 < num_chunks { i + 1 } else { SLAB_NONE };
        }

        self.head_with_space = if num_chunks > 0 { 0 } else { SLAB_NONE };
        self.len = 0;
    }
}

// =============================================================================
// Traits
// =============================================================================

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
            .field("num_chunks", &self.slabs.len())
            .field("chunk_capacity", &self.chunk_capacity)
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
/// let mut slab = Slab::with_chunk_capacity(1024);
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
    chunk_idx: u32,
    local_idx: u32,
    inserted: bool,
}

impl<'a, T> VacantEntry<'a, T> {
    /// Returns the key that will be associated with the inserted value.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.slab.encode(self.chunk_idx, self.local_idx))
    }

    /// Inserts a value into the reserved slot, returning the key.
    #[inline]
    pub fn insert(mut self, value: T) -> Key {
        let key = self.key();
        let entry = self.slab.entry_mut(self.chunk_idx);

        unsafe {
            entry.inner.fill_reserved(self.local_idx, value);
        }

        if entry.inner.is_full() {
            self.slab.head_with_space = entry.next_with_space;
        }

        self.slab.len += 1;
        self.inserted = true;
        key
    }
}

impl<T> Drop for VacantEntry<'_, T> {
    fn drop(&mut self) {
        if !self.inserted {
            let entry = self.slab.entry_mut(self.chunk_idx);
            unsafe {
                entry.inner.cancel_reserved(self.local_idx);
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
    fn empty_on_construction() {
        let slab: Slab<u64> = Slab::with_chunk_capacity(16);
        assert!(slab.is_empty());
        assert_eq!(slab.len(), 0);
        assert_eq!(slab.capacity(), 0);
    }

    #[test]
    fn insert_get_remove() {
        let mut slab = Slab::with_chunk_capacity(16);

        let key = slab.insert(42u64);
        assert_eq!(slab.len(), 1);
        assert_eq!(slab.get(key), Some(&42));

        let removed = slab.remove(key);
        assert_eq!(removed, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn grows_automatically() {
        let mut slab = Slab::with_chunk_capacity(4);

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
        let mut slab = Slab::with_chunk_capacity(16);

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

        let mut slab = Slab::with_chunk_capacity(16);

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
        let mut slab = Slab::<usize>::with_chunk_capacity(16);

        let key = {
            let entry = slab.vacant_entry();
            entry.key()
        };

        assert_eq!(slab.len(), 0);
        assert_eq!(slab.get(key), None);
    }

    #[test]
    fn clear_preserves_capacity() {
        let mut slab = Slab::with_chunk_capacity(16);

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
        let slab: Slab<u64> = Slab::with_chunk_capacity(16);

        for index in [0, 1, 15, 16, 31, 32, 47, 48, 100, 1000] {
            let (chunk_idx, local_idx) = slab.decode(index);
            let encoded = slab.encode(chunk_idx, local_idx);
            assert_eq!(encoded, index, "roundtrip failed for {}", index);
        }
    }

    #[test]
    fn fixed_chunk_capacity() {
        let mut slab: Slab<u64> = Slab::with_chunk_capacity(16);

        // Fill first chunk
        for i in 0..16u64 {
            slab.insert(i);
        }
        assert_eq!(slab.num_chunks(), 1);
        assert_eq!(slab.capacity(), 16);

        // Trigger second chunk
        slab.insert(16);
        assert_eq!(slab.num_chunks(), 2);
        assert_eq!(slab.capacity(), 32);

        // Trigger third chunk
        for i in 17..33u64 {
            slab.insert(i);
        }
        assert_eq!(slab.num_chunks(), 3);
        assert_eq!(slab.capacity(), 48);
    }

    #[test]
    fn slab_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Slab<u64>>();
        assert_send::<Slab<String>>();
    }

    #[test]
    fn stress_insert_remove() {
        let mut slab = Slab::with_chunk_capacity(16);
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
