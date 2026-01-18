//! Growable slab allocator.
//!
//! [`Slab`] provides O(1) insert, access, and remove. Grows automatically
//! by adding fixed-size chunks when capacity is exceeded.

use std::{
    marker::PhantomData,
    ops::{Index, IndexMut},
    pin::Pin,
};

use crate::{BoundedSlab, Key};

const SLAB_NONE: u32 = u32::MAX;
const DEFAULT_CHUNK_CAPACITY: usize = 4096;

// =============================================================================
// SlabBuilder
// =============================================================================

/// Builder for configuring a [`Slab`].
///
/// # Example
///
/// ```
/// use nexus_slab::Slab;
///
/// let slab: Slab<u64> = Slab::builder()
///     .chunk_capacity(8192)
///     .reserve(100_000)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct SlabBuilder<T> {
    chunk_capacity: usize,
    reserve: usize,
    _marker: PhantomData<T>,
}

impl<T> Default for SlabBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SlabBuilder<T> {
    /// Creates a new builder with default settings.
    pub fn new() -> Self {
        Self {
            chunk_capacity: DEFAULT_CHUNK_CAPACITY,
            reserve: 0,
            _marker: PhantomData,
        }
    }

    /// Sets the capacity of each internal chunk.
    ///
    /// Rounded up to the next power of two. Default: 4096.
    ///
    /// Smaller chunks = less memory waste, more growth events.
    /// Larger chunks = fewer growth events, more memory per allocation.
    pub fn chunk_capacity(mut self, capacity: usize) -> Self {
        self.chunk_capacity = capacity;
        self
    }

    /// Pre-allocates space for at least this many items.
    ///
    /// Allocates enough chunks to hold `count` items. Default: 0 (lazy).
    pub fn reserve(mut self, count: usize) -> Self {
        self.reserve = count;
        self
    }

    /// Builds the slab.
    pub fn build(self) -> Slab<T> {
        let mut slab = Slab::with_chunk_capacity(self.chunk_capacity);
        while slab.capacity() < self.reserve {
            slab.grow();
        }
        slab
    }
}

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
#[repr(C)]
pub struct Slab<T> {
    slabs: Vec<SlabEntry<T>>,
    chunk_shift: u32,
    chunk_mask: u32,
    head_with_space: u32,
    len: usize,
    chunk_capacity: u32,
}

impl<T> Slab<T> {
    // =========================================================================
    // Construction
    // =========================================================================

    /// Creates a new empty slab with default settings.
    ///
    /// Uses a chunk capacity of 4096 slots. No memory is allocated
    /// until the first insert.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::Slab;
    ///
    /// let mut slab: Slab<u64> = Slab::new();
    /// let key = slab.insert(42);
    /// ```
    pub fn new() -> Self {
        Self::with_chunk_capacity(DEFAULT_CHUNK_CAPACITY)
    }

    /// Creates a new slab with pre-allocated capacity.
    ///
    /// Uses the default chunk capacity (4096 slots) and allocates
    /// enough chunks to hold at least `capacity` items.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::Slab;
    ///
    /// let slab: Slab<u64> = Slab::with_capacity(10_000);
    /// assert!(slab.capacity() >= 10_000);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self::builder().reserve(capacity).build()
    }

    /// Returns a builder for configuring a slab.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::Slab;
    ///
    /// let slab: Slab<u64> = Slab::builder()
    ///     .chunk_capacity(8192)
    ///     .reserve(50_000)
    ///     .build();
    /// ```
    pub fn builder() -> SlabBuilder<T> {
        SlabBuilder::new()
    }

    /// Creates a new slab with the specified chunk capacity.
    ///
    /// For most uses, prefer [`new`](Self::new), [`with_capacity`](Self::with_capacity),
    /// or [`builder`](Self::builder).
    ///
    /// Chunk capacity is rounded up to the next power of two.
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

    /// Returns a pinned mutable reference to the value.
    ///
    /// This is safe because values in the slab have a stable address
    /// until removed, and removal requires `&mut self` which conflicts
    /// with the returned reference.
    #[inline]
    pub fn get_pinned(&mut self, key: Key) -> Option<Pin<&mut T>> {
        self.get_mut(key).map(|r| unsafe { Pin::new_unchecked(r) })
    }

    /// Returns a pinned mutable reference without validation.
    ///
    /// # Safety
    ///
    /// The key must refer to a valid, occupied slot.
    #[inline]
    pub unsafe fn get_pinned_unchecked(&mut self, key: Key) -> Pin<&mut T> {
        unsafe { Pin::new_unchecked(self.get_unchecked_mut(key)) }
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

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<T: Send> Send for Slab<T> {}

impl<T> Index<Key> for Slab<T> {
    type Output = T;

    fn index(&self, key: Key) -> &Self::Output {
        self.get(key).expect("invalid key")
    }
}

impl<T> IndexMut<Key> for Slab<T> {
    fn index_mut(&mut self, key: Key) -> &mut Self::Output {
        self.get_mut(key).expect("invalid key")
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

    // =========================================================================
    // LIFO Behavior
    // =========================================================================

    #[test]
    fn insert_after_remove_reuses_slot_lifo() {
        let mut slab = Slab::with_capacity(100);

        let _k1 = slab.insert(1u64);
        let k2 = slab.insert(2u64);
        let _k3 = slab.insert(3u64);

        slab.remove(k2);

        let k4 = slab.insert(4u64);
        assert_eq!(k4.index(), k2.index()); // LIFO reuse
        assert_eq!(slab[k4], 4);
    }

    #[test]
    fn freelist_chain_works_correctly() {
        let mut slab = Slab::with_capacity(100);

        let _k1 = slab.insert(1u64);
        let k2 = slab.insert(2u64);
        let k3 = slab.insert(3u64);
        let _k4 = slab.insert(4u64);

        // Remove in order: k2, k3 (builds chain k3 -> k2)
        slab.remove(k2);
        slab.remove(k3);

        // Insert should get k3 first (LIFO), then k2
        let new1 = slab.insert(10u64);
        let new2 = slab.insert(20u64);

        assert_eq!(new1.index(), k3.index());
        assert_eq!(new2.index(), k2.index());
    }

    #[test]
    fn chunk_freelist_lifo_on_remove() {
        let mut slab: Slab<u64> = Slab::builder().chunk_capacity(16).build();

        // Fill first chunk completely, spill into second
        let mut keys = Vec::new();
        for i in 0..20u64 {
            keys.push(slab.insert(i));
        }

        assert_eq!(slab.num_chunks(), 2);

        // First 16 keys are in chunk 0, next 4 in chunk 1
        // Remove from chunk 0 (was full) - should push chunk 0 back to freelist
        let k0 = keys[0];
        slab.remove(k0);

        // Next insert should reuse the freed slot (LIFO within chunk 0)
        let new_key = slab.insert(999);
        assert_eq!(new_key.index(), k0.index());
        assert_eq!(slab[new_key], 999);
    }

    // =========================================================================
    // Growth Behavior
    // =========================================================================

    #[test]
    fn growth_preserves_existing_values() {
        let mut slab: Slab<u64> = Slab::builder().chunk_capacity(16).build();

        let mut keys = Vec::new();
        for i in 0..16u64 {
            keys.push(slab.insert(i));
        }

        assert_eq!(slab.num_chunks(), 1);

        // Force growth
        for i in 16..100u64 {
            slab.insert(i);
        }

        assert!(slab.num_chunks() > 1);

        // Verify original values unchanged
        for (i, &k) in keys.iter().enumerate() {
            assert_eq!(slab[k], i as u64);
        }
    }

    // =========================================================================
    // No Double Allocation
    // =========================================================================

    #[test]
    fn no_double_allocation() {
        use std::collections::HashSet;

        let mut slab: Slab<u64> = Slab::builder().chunk_capacity(32).build();
        let mut allocated: HashSet<u32> = HashSet::new();

        let mut keys = Vec::new();
        for i in 0..200u64 {
            let k = slab.insert(i);
            assert!(
                !allocated.contains(&k.index()),
                "Double allocation on insert: {}",
                k.index()
            );
            allocated.insert(k.index());
            keys.push(k);
        }

        // Remove half
        for i in (0..200).step_by(2) {
            let k = keys[i];
            allocated.remove(&k.index());
            slab.remove(k);
        }

        // Insert more
        for i in 0..100u64 {
            let k = slab.insert(1000 + i);
            assert!(
                !allocated.contains(&k.index()),
                "Double allocation on reinsert: {}",
                k.index()
            );
            allocated.insert(k.index());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn no_double_allocation_stress() {
        use std::collections::HashMap;

        let mut slab: Slab<u64> = Slab::builder().chunk_capacity(64).build();
        let mut live: HashMap<u32, u64> = HashMap::new();

        for round in 0..100 {
            for i in 0..50 {
                let val = (round * 1000 + i) as u64;
                let k = slab.insert(val);

                if let Some(old) = live.get(&k.index()) {
                    panic!(
                        "Double allocation: index {} has {}, inserting {}",
                        k.index(),
                        old,
                        val
                    );
                }
                live.insert(k.index(), val);
            }

            let to_remove: Vec<_> = live.keys().take(25).cloned().collect();
            for idx in to_remove {
                let k = Key::new(idx);
                let val = slab.remove(k);
                let expected = live.remove(&idx).unwrap();
                assert_eq!(val, expected);
            }
        }
    }

    // =========================================================================
    // Drop Behavior
    // =========================================================================

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

        let mut slab = Slab::with_capacity(100);
        for _ in 0..50 {
            slab.insert(DropCounter);
        }

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);

        slab.clear();

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 50);
    }

    #[test]
    fn drop_cleans_up_all_chunks() {
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
            let mut slab: Slab<DropCounter> = Slab::builder().chunk_capacity(16).build();

            for _ in 0..50 {
                slab.insert(DropCounter);
            }

            assert!(slab.num_chunks() >= 3);
        }

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 50);
    }

    // =========================================================================
    // Edge Cases
    // =========================================================================

    #[test]
    fn zero_sized_type() {
        let mut slab = Slab::<()>::with_capacity(100);

        let mut keys = Vec::new();
        for _ in 0..50 {
            keys.push(slab.insert(()));
        }

        assert_eq!(slab.len(), 50);

        for k in keys {
            slab.remove(k);
        }

        assert!(slab.is_empty());
    }

    #[test]
    fn large_value_type() {
        #[derive(Clone, PartialEq, Debug)]
        struct Large([u64; 64]); // 512 bytes

        let mut slab = Slab::with_capacity(16);

        let val = Large([42; 64]);
        let k = slab.insert(val.clone());

        assert_eq!(slab[k], val);
    }

    // =========================================================================
    // Stress Tests (extended)
    // =========================================================================

    #[test]
    #[cfg_attr(miri, ignore)]
    fn stress_random_operations() {
        use std::collections::HashMap;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn pseudo_random(seed: u64) -> u64 {
            let mut hasher = DefaultHasher::new();
            seed.hash(&mut hasher);
            hasher.finish()
        }

        let mut slab: Slab<u64> = Slab::builder().chunk_capacity(64).build();
        let mut live: HashMap<u32, u64> = HashMap::new();
        let mut seed = 12345u64;

        for _ in 0..10000 {
            seed = pseudo_random(seed);

            if live.is_empty() || seed % 3 != 0 {
                let val = seed;
                let k = slab.insert(val);
                live.insert(k.index(), val);
            } else {
                let idx = (seed as usize) % live.len();
                let &index = live.keys().nth(idx).unwrap();
                let k = Key::new(index);
                let val = slab.remove(k);
                let expected = live.remove(&index).unwrap();
                assert_eq!(val, expected);
            }
        }

        assert_eq!(slab.len(), live.len());
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn stress_insert_remove_cycles() {
        use std::collections::HashMap;

        let mut slab: Slab<u64> = Slab::builder().chunk_capacity(64).build();
        let mut keys: Vec<Key> = Vec::new();
        let mut expected: HashMap<u32, u64> = HashMap::new();

        for cycle in 0..100 {
            for i in 0..100 {
                let val = (cycle * 1000 + i) as u64;
                let k = slab.insert(val);
                keys.push(k);
                expected.insert(k.index(), val);
            }

            // Verify all values
            for (&idx, &val) in &expected {
                let k = Key::new(idx);
                assert_eq!(slab[k], val);
            }

            // Remove half
            let drain_count = keys.len() / 2;
            for _ in 0..drain_count {
                let k = keys.pop().unwrap();
                let val = slab.remove(k);
                let expected_val = expected.remove(&k.index()).unwrap();
                assert_eq!(val, expected_val);
            }
        }
    }
}
