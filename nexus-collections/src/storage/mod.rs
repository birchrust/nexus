//! Storage backends for slab-like containers with stable keys.
//!
//! This module provides storage implementations for the collection types.
//! Storage owns the data and provides stable keys for access.
//!
//! # Specialized Storage Types
//!
//! Each collection has dedicated storage types that hide internal node structure:
//!
//! | Collection | Bounded | Growable |
//! |------------|---------|----------|
//! | [`List`](crate::List) | [`ListStorage<T>`] | [`GrowableListStorage<T>`] |
//!
//! # Legacy Storage (Deprecated)
//!
//! The generic [`Storage`] trait and [`BoxedStorage`] are deprecated and will
//! be removed in a future version. Prefer the specialized storage types above.
//!
//! # Example
//!
//! ```
//! use nexus_collections::{List, ListStorage};
//!
//! // Create storage - capacity is the only parameter
//! let mut storage: ListStorage<u64> = ListStorage::with_capacity(1000);
//!
//! // Use with List
//! let mut list: List<u64, ListStorage<u64>, _> = List::new();
//! let key = list.try_push_back(&mut storage, 42).unwrap();
//! assert_eq!(list.get(&storage, key), Some(&42));
//! ```

mod boxed;
mod list;
mod slab;

// Re-export specialized storage types
pub use list::{GrowableListStorage, ListNode, ListStorage};

// Re-export legacy types (to be deprecated in Phase 5)
pub use boxed::BoxedStorage;

use crate::Key;

// =============================================================================
// Traits (Legacy - will be removed in Phase 5)
// =============================================================================

/// Base storage trait with stable keys.
///
/// Provides common operations for slab-like containers. Keys remain
/// valid until explicitly removed, enabling node-based data structures
/// to use keys instead of pointers.
///
/// See [`BoundedStorage`] for fixed-capacity storage and
/// [`UnboundedStorage`] for growable storage.
///
/// # Deprecated
///
/// This trait will be removed in a future version. Use the specialized
/// storage types ([`ListStorage`], etc.) instead.
pub trait Storage<T> {
    /// Key type for this storage.
    type Key: Key;

    /// Removes and returns the value at `key`, if present.
    fn remove(&mut self, key: Self::Key) -> Option<T>;

    /// Returns a reference to the value at `key`, if present.
    fn get(&self, key: Self::Key) -> Option<&T>;

    /// Returns a mutable reference to the value at `key`, if present.
    fn get_mut(&mut self, key: Self::Key) -> Option<&mut T>;

    /// Returns the number of occupied slots.
    fn len(&self) -> usize;

    /// Returns `true` if no slots are occupied.
    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a reference without bounds checking.
    ///
    /// # Safety
    ///
    /// `key` must be valid and occupied.
    unsafe fn get_unchecked(&self, key: Self::Key) -> &T;

    /// Returns a mutable reference without bounds checking.
    ///
    /// # Safety
    ///
    /// `key` must be valid and occupied.
    unsafe fn get_unchecked_mut(&mut self, key: Self::Key) -> &mut T;

    /// Removes an element without bounds checking.
    ///
    /// # Safety
    ///
    /// The key must be valid and occupied.
    unsafe fn remove_unchecked(&mut self, key: Self::Key) -> T;
}

/// Fixed-capacity storage where insertion can fail.
///
/// Use this for pre-allocated, bounded storage where capacity is known
/// upfront. Insertion returns `Result<Key, Full<T>>`.
///
/// # Deprecated
///
/// This trait will be removed in a future version. Use the specialized
/// storage types ([`ListStorage`], etc.) instead.
pub trait BoundedStorage<T>: Storage<T> {
    /// Attempts to insert a value, returning its stable key.
    ///
    /// Returns `Err(Full(value))` if storage is at capacity.
    fn try_insert(&mut self, value: T) -> Result<Self::Key, Full<T>>;

    /// Returns the total capacity (number of slots).
    fn capacity(&self) -> usize;

    /// Returns `true` if all slots are occupied.
    #[inline]
    fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }
}

/// Growable storage where insertion always succeeds.
///
/// Use this when you want storage that grows as needed. Insertion
/// is infallible (may allocate).
///
/// # Deprecated
///
/// This trait will be removed in a future version. Use the specialized
/// storage types ([`GrowableListStorage`], etc.) instead.
pub trait UnboundedStorage<T>: Storage<T> {
    /// Inserts a value, returning its stable key.
    ///
    /// This operation is infallible but may allocate.
    fn insert(&mut self, value: T) -> Self::Key;
}

// =============================================================================
// Error Type
// =============================================================================

/// Error returned when fixed-capacity storage is full.
///
/// Contains the value that could not be inserted, allowing recovery.
/// Modeled after `std::sync::mpsc::SendError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full<T>(pub T);

impl<T> Full<T> {
    /// Returns the value that could not be inserted.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> core::fmt::Display for Full<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "storage is full")
    }
}

impl<T: core::fmt::Debug> std::error::Error for Full<T> {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let storage: BoxedStorage<u64> = BoxedStorage::with_capacity(16);
        assert!(storage.is_empty());
        assert!(!storage.is_full());
        assert_eq!(storage.len(), 0);
        assert_eq!(storage.capacity(), 16);
    }

    #[test]
    fn capacity_rounds_to_power_of_two() {
        let storage: BoxedStorage<u64> = BoxedStorage::with_capacity(100);
        assert_eq!(storage.capacity(), 128);

        let storage: BoxedStorage<u64> = BoxedStorage::with_capacity(1000);
        assert_eq!(storage.capacity(), 1024);
    }

    #[test]
    fn insert_get_remove() {
        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(16);

        let key = storage.try_insert(42).unwrap();
        assert_eq!(storage.len(), 1);
        assert_eq!(storage.get(key), Some(&42));

        let removed = storage.remove(key);
        assert_eq!(removed, Some(42));
        assert_eq!(storage.get(key), None);
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn get_mut() {
        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(16);

        let key = storage.try_insert(10).unwrap();
        *storage.get_mut(key).unwrap() = 20;

        assert_eq!(storage.get(key), Some(&20));
    }

    #[test]
    fn fill_to_capacity() {
        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(4);

        let k0 = storage.try_insert(0).unwrap();
        let k1 = storage.try_insert(1).unwrap();
        let k2 = storage.try_insert(2).unwrap();
        let k3 = storage.try_insert(3).unwrap();

        assert!(storage.is_full());

        let err = storage.try_insert(4);
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().into_inner(), 4);

        assert_eq!(storage.get(k0), Some(&0));
        assert_eq!(storage.get(k1), Some(&1));
        assert_eq!(storage.get(k2), Some(&2));
        assert_eq!(storage.get(k3), Some(&3));
    }

    #[test]
    fn slot_reuse() {
        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(4);

        let k0 = storage.try_insert(0).unwrap();
        let _k1 = storage.try_insert(1).unwrap();

        storage.remove(k0);

        // Next insert reuses k0's slot (LIFO)
        let k2 = storage.try_insert(2).unwrap();
        assert_eq!(k2, k0);
    }

    #[test]
    fn remove_nonexistent() {
        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(16);

        let key = storage.try_insert(42).unwrap();
        storage.remove(key);

        // Double remove returns None
        assert_eq!(storage.remove(key), None);
    }

    #[test]
    fn clear_storage() {
        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(16);

        storage.try_insert(1).unwrap();
        storage.try_insert(2).unwrap();
        storage.try_insert(3).unwrap();

        assert_eq!(storage.len(), 3);

        storage.clear();

        assert_eq!(storage.len(), 0);
        assert!(storage.is_empty());
        assert!(!storage.is_full());
    }

    #[test]
    fn drop_cleans_up() {
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
            let mut storage: BoxedStorage<DropCounter> = BoxedStorage::with_capacity(8);
            storage.try_insert(DropCounter).unwrap();
            storage.try_insert(DropCounter).unwrap();
            storage.try_insert(DropCounter).unwrap();
        }

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn large_capacity() {
        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(4096);
        assert_eq!(storage.capacity(), 4096);

        // Fill it
        let mut keys = Vec::with_capacity(4096);
        for i in 0..4096 {
            keys.push(storage.try_insert(i as u64).unwrap());
        }
        assert!(storage.is_full());

        // Verify all values
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(storage.get(*key), Some(&(i as u64)));
        }
    }

    // =========================================================================
    // Benchmarks
    // =========================================================================

    #[test]
    #[ignore]
    fn bench_boxed_storage() {
        use std::time::Instant;

        const CAPACITY: usize = 4096;
        const ITERATIONS: usize = 100_000;

        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(CAPACITY);

        // Warmup
        for i in 0..CAPACITY {
            storage.try_insert(i as u64).unwrap();
        }
        for i in 0..CAPACITY {
            storage.remove(i);
        }

        // Collect timings
        let mut insert_ns = Vec::with_capacity(ITERATIONS);
        let mut get_ns = Vec::with_capacity(ITERATIONS);
        let mut remove_ns = Vec::with_capacity(ITERATIONS);

        for i in 0..ITERATIONS {
            // Insert
            let start = Instant::now();
            let key = storage.try_insert(i as u64).unwrap();
            insert_ns.push(start.elapsed().as_nanos() as u64);

            // Get
            let start = Instant::now();
            let _ = std::hint::black_box(storage.get(key));
            get_ns.push(start.elapsed().as_nanos() as u64);

            // Remove
            let start = Instant::now();
            let _ = std::hint::black_box(storage.remove(key));
            remove_ns.push(start.elapsed().as_nanos() as u64);
        }

        // Sort for percentiles
        insert_ns.sort_unstable();
        get_ns.sort_unstable();
        remove_ns.sort_unstable();

        fn percentile(sorted: &[u64], p: f64) -> u64 {
            let idx = ((p / 100.0) * sorted.len() as f64) as usize;
            sorted[idx.min(sorted.len() - 1)]
        }

        fn print_stats(name: &str, sorted: &[u64]) {
            println!(
                "{:8} | p50: {:4} ns | p90: {:4} ns | p99: {:4} ns | p999: {:5} ns",
                name,
                percentile(sorted, 50.0),
                percentile(sorted, 90.0),
                percentile(sorted, 99.0),
                percentile(sorted, 99.9),
            );
        }

        println!(
            "\nBoxedStorage<u64> ({} iterations, capacity {})",
            ITERATIONS, CAPACITY
        );
        println!("---------------------------------------------------------");
        print_stats("insert", &insert_ns);
        print_stats("get", &get_ns);
        print_stats("remove", &remove_ns);
        println!();
    }

    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    #[test]
    #[ignore]
    fn bench_boxed_storage_tsc() {
        const CAPACITY: usize = 4096;
        const ITERATIONS: usize = 100_000;

        #[inline]
        fn rdtsc() -> u64 {
            unsafe {
                core::arch::x86_64::_mm_lfence();
                core::arch::x86_64::_rdtsc()
            }
        }

        let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(CAPACITY);

        // Warmup
        for i in 0..CAPACITY {
            storage.try_insert(i as u64).unwrap();
        }
        for i in 0..CAPACITY {
            storage.remove(i);
        }

        // Collect timings
        let mut insert_cycles = Vec::with_capacity(ITERATIONS);
        let mut get_cycles = Vec::with_capacity(ITERATIONS);
        let mut remove_cycles = Vec::with_capacity(ITERATIONS);

        for i in 0..ITERATIONS {
            // Insert
            let start = rdtsc();
            let key = storage.try_insert(i as u64).unwrap();
            let end = rdtsc();
            insert_cycles.push(end - start);

            // Get
            let start = rdtsc();
            let _ = std::hint::black_box(storage.get(key));
            let end = rdtsc();
            get_cycles.push(end - start);

            // Remove
            let start = rdtsc();
            let _ = std::hint::black_box(storage.remove(key));
            let end = rdtsc();
            remove_cycles.push(end - start);
        }

        // Sort for percentiles
        insert_cycles.sort_unstable();
        get_cycles.sort_unstable();
        remove_cycles.sort_unstable();

        fn percentile(sorted: &[u64], p: f64) -> u64 {
            let idx = ((p / 100.0) * sorted.len() as f64) as usize;
            sorted[idx.min(sorted.len() - 1)]
        }

        fn print_stats(name: &str, sorted: &[u64]) {
            println!(
                "{:8} | p50: {:5} cycles | p90: {:5} cycles | p99: {:5} cycles | p999: {:6} cycles",
                name,
                percentile(sorted, 50.0),
                percentile(sorted, 90.0),
                percentile(sorted, 99.0),
                percentile(sorted, 99.9),
            );
        }

        println!(
            "\nBoxedStorage<u64> ({} iterations, capacity {})",
            ITERATIONS, CAPACITY
        );
        println!("------------------------------------------------------------------------");
        print_stats("insert", &insert_cycles);
        print_stats("get", &get_cycles);
        print_stats("remove", &remove_cycles);
        println!();
    }
}
