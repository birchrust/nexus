//! BoxedStorage - runtime capacity, single allocation, bitmap occupancy.
//!
//! This is a legacy storage type that will be removed in a future version.
//! Prefer using the specialized storage types like [`ListStorage`](super::ListStorage).

use super::{BoundedStorage, Full, Storage};

use core::mem::MaybeUninit;
use core::ptr::NonNull;
use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::marker::PhantomData;

const NONE: usize = usize::MAX;

/// Fixed-capacity storage with runtime-determined size.
///
/// Uses a single heap allocation containing:
/// - Entry array (`MaybeUninit<T>`)
/// - Occupancy bitmap (`u64` words)
/// - Free stack (keys)
///
/// Capacity is rounded up to the next power of 2 for bitmap efficiency.
///
/// # Deprecated
///
/// This type will be removed in a future version. Use the specialized
/// storage types ([`ListStorage`](super::ListStorage), etc.) instead.
///
/// # Example
///
/// ```
/// use nexus_collections::{BoxedStorage, BoundedStorage, Storage};
///
/// let mut storage: BoxedStorage<u64> = BoxedStorage::with_capacity(1000);
/// assert!(storage.capacity() >= 1000); // Rounded to 1024
///
/// let key = storage.try_insert(42).unwrap();
/// assert_eq!(storage.get(key), Some(&42));
/// ```
pub struct BoxedStorage<T> {
    /// Single allocation containing entries, bitmap, and free stack.
    ptr: NonNull<u8>,
    /// Capacity (always power of 2).
    capacity: usize,
    /// Number of free slots.
    free_len: usize,
    /// Cached layout for deallocation.
    layout: Layout,
    /// Offset to bitmap from ptr.
    bitmap_offset: usize,
    /// Offset to free stack from ptr.
    free_stack_offset: usize,
    _marker: PhantomData<T>,
}

impl<T> BoxedStorage<T> {
    /// Creates storage with at least `min_capacity` slots.
    ///
    /// Actual capacity is rounded up to the next power of 2.
    ///
    /// # Panics
    ///
    /// Panics if `min_capacity` is 0 or exceeds the key type's maximum.
    pub fn with_capacity(min_capacity: usize) -> Self {
        assert!(min_capacity > 0, "capacity must be > 0");

        // Round up to power of 2 for bitmap efficiency
        let capacity = min_capacity.next_power_of_two();

        // Note: NONE is usize::MAX, so this check guards against overflow from next_power_of_two()
        #[allow(clippy::absurd_extreme_comparisons)]
        {
            assert!(capacity <= NONE, "capacity exceeds key type maximum");
        }

        // Calculate layout
        // Layout: [entries][padding][bitmap][padding][free_stack]
        let entries_layout = Layout::array::<MaybeUninit<T>>(capacity).unwrap();
        let bitmap_words = bitmap_words(capacity);
        let bitmap_layout = Layout::array::<u64>(bitmap_words).unwrap();
        let free_stack_layout = Layout::array::<usize>(capacity).unwrap();

        let (layout, bitmap_offset) = entries_layout.extend(bitmap_layout).unwrap();
        let (layout, free_stack_offset) = layout.extend(free_stack_layout).unwrap();
        let layout = layout.pad_to_align();

        // Allocate
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            handle_alloc_error(layout);
        }
        let ptr = unsafe { NonNull::new_unchecked(ptr) };

        // Initialize bitmap to all zeros (all slots free, but we track via free_stack)
        unsafe {
            let bitmap_ptr = ptr.as_ptr().add(bitmap_offset) as *mut u64;
            core::ptr::write_bytes(bitmap_ptr, 0, bitmap_words);
        }

        // Initialize free stack
        unsafe {
            let free_stack_ptr = ptr.as_ptr().add(free_stack_offset) as *mut usize;
            for i in 0..capacity {
                free_stack_ptr.add(i).write(i);
            }
        }

        Self {
            ptr,
            capacity,
            free_len: capacity,
            layout,
            bitmap_offset,
            free_stack_offset,
            _marker: PhantomData,
        }
    }

    /// Removes all elements from storage.
    ///
    /// This drops all stored values and makes all slots available for reuse.
    ///
    /// # Warning
    ///
    /// If any data structures (List, Heap, etc.) still reference keys in
    /// this storage, they will have dangling references. Only call this when
    /// you know nothing else references the storage, or after clearing those
    /// data structures first.
    pub fn clear(&mut self) {
        // Drop all occupied values
        for i in 0..self.capacity {
            if self.is_occupied(i) {
                // Safety: slot is occupied
                unsafe {
                    let ptr = self.entries_ptr().add(i);
                    std::ptr::drop_in_place((*ptr).as_mut_ptr());
                }
            }
        }

        // Reset bitmap to all zeros (all vacant)
        unsafe {
            std::ptr::write_bytes(self.bitmap_ptr(), 0, bitmap_words(self.capacity));
        }

        // Rebuild free stack
        let free_stack = self.free_stack_ptr();
        for i in 0..self.capacity {
            unsafe {
                *free_stack.add(i) = i;
            }
        }
        self.free_len = self.capacity;
    }

    #[inline]
    fn entries_ptr(&self) -> *mut MaybeUninit<T> {
        self.ptr.as_ptr() as *mut MaybeUninit<T>
    }

    #[inline]
    fn bitmap_ptr(&self) -> *mut u64 {
        unsafe { self.ptr.as_ptr().add(self.bitmap_offset) as *mut u64 }
    }

    #[inline]
    fn free_stack_ptr(&self) -> *mut usize {
        unsafe { self.ptr.as_ptr().add(self.free_stack_offset) as *mut usize }
    }

    #[inline]
    fn is_occupied(&self, idx: usize) -> bool {
        let word = idx / 64;
        let bit = idx % 64;
        unsafe {
            let bitmap = self.bitmap_ptr();
            (*bitmap.add(word) & (1 << bit)) != 0
        }
    }

    #[inline]
    fn set_occupied(&mut self, idx: usize) {
        let word = idx / 64;
        let bit = idx % 64;
        unsafe {
            let bitmap = self.bitmap_ptr();
            *bitmap.add(word) |= 1 << bit;
        }
    }

    #[inline]
    fn set_vacant(&mut self, idx: usize) {
        let word = idx / 64;
        let bit = idx % 64;
        unsafe {
            let bitmap = self.bitmap_ptr();
            *bitmap.add(word) &= !(1 << bit);
        }
    }
}

impl<T> Storage<T> for BoxedStorage<T> {
    type Key = usize;

    #[inline]
    fn remove(&mut self, key: Self::Key) -> Option<T> {
        if key >= self.capacity || !self.is_occupied(key) {
            return None;
        }

        self.set_vacant(key);
        let value = unsafe { self.entries_ptr().add(key).read().assume_init() };

        unsafe {
            self.free_stack_ptr().add(self.free_len).write(key);
        }
        self.free_len += 1;

        Some(value)
    }

    #[inline]
    fn get(&self, key: Self::Key) -> Option<&T> {
        if key >= self.capacity || !self.is_occupied(key) {
            return None;
        }

        Some(unsafe { (*self.entries_ptr().add(key)).assume_init_ref() })
    }

    #[inline]
    fn get_mut(&mut self, key: Self::Key) -> Option<&mut T> {
        if key >= self.capacity || !self.is_occupied(key) {
            return None;
        }

        Some(unsafe { (*self.entries_ptr().add(key)).assume_init_mut() })
    }

    #[inline]
    fn len(&self) -> usize {
        self.capacity - self.free_len
    }

    #[inline]
    unsafe fn get_unchecked(&self, key: Self::Key) -> &T {
        unsafe { (*self.entries_ptr().add(key)).assume_init_ref() }
    }

    #[inline]
    unsafe fn get_unchecked_mut(&mut self, key: Self::Key) -> &mut T {
        unsafe { (*self.entries_ptr().add(key)).assume_init_mut() }
    }

    #[inline]
    unsafe fn remove_unchecked(&mut self, key: Self::Key) -> T {
        self.set_vacant(key);
        let value = unsafe { self.entries_ptr().add(key).read().assume_init() };

        unsafe {
            self.free_stack_ptr().add(self.free_len).write(key);
        }
        self.free_len += 1;

        value
    }
}

impl<T> BoundedStorage<T> for BoxedStorage<T> {
    #[inline]
    fn try_insert(&mut self, value: T) -> Result<Self::Key, Full<T>> {
        if self.free_len == 0 {
            return Err(Full(value));
        }

        self.free_len -= 1;
        let key = unsafe { *self.free_stack_ptr().add(self.free_len) };

        unsafe {
            self.entries_ptr().add(key).write(MaybeUninit::new(value));
        }
        self.set_occupied(key);

        Ok(key)
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<T> Drop for BoxedStorage<T> {
    fn drop(&mut self) {
        // Drop all occupied entries
        for i in 0..self.capacity {
            if self.is_occupied(i) {
                unsafe {
                    self.entries_ptr().add(i).read().assume_init_drop();
                }
            }
        }

        // Deallocate
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

// Safety: BoxedStorage owns its data, safe to send if T is Send
unsafe impl<T: Send> Send for BoxedStorage<T> {}

// =============================================================================
// Helper functions
// =============================================================================

#[inline]
const fn bitmap_words(capacity: usize) -> usize {
    capacity.div_ceil(64)
}
