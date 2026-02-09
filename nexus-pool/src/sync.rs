//! Single-acquirer pool: one thread acquires, any thread can return.
//!
//! Items are acquired from a single point (the `Acquirer`) and can be
//! returned from any thread via `Drop` on `Pooled`.
//!
//! Uses LIFO ordering for cache locality.

use std::cell::UnsafeCell;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Weak};

const NONE: u32 = u32::MAX;

// =============================================================================
// Slot - individual pool entry
// =============================================================================

struct Slot<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    next: AtomicU32,
}

// Safety: Slot is Send+Sync because:
// - value is only accessed when slot is "owned" (popped from free list)
// - next is atomic
unsafe impl<T: Send> Send for Slot<T> {}
unsafe impl<T: Send + Sync> Sync for Slot<T> {}

// =============================================================================
// Inner - shared pool state
// =============================================================================

struct Inner<T> {
    slots: Box<[Slot<T>]>,
    free_head: AtomicU32,
    reset: Box<dyn Fn(&mut T) + Send + Sync>,
}

impl<T> Inner<T> {
    /// Push a slot back onto the free list. Called from any thread.
    fn push(&self, idx: u32, mut value: T) {
        // Reset the value
        (self.reset)(&mut value);

        // Write value back to slot
        // Safety: we own this slot (it was popped), no one else can access it
        unsafe {
            (*self.slots[idx as usize].value.get()).write(value);
        }

        // Link into free list with CAS loop
        loop {
            let head = self.free_head.load(Ordering::Relaxed);
            self.slots[idx as usize].next.store(head, Ordering::Relaxed);

            match self.free_head.compare_exchange_weak(
                head,
                idx,
                Ordering::Release, // Publishes value write + next write
                Ordering::Relaxed, // Failure just retries
            ) {
                Ok(_) => return,
                Err(_) => std::hint::spin_loop(),
            }
        }
    }

    /// Pop a slot from the free list. Called only from Acquirer thread.
    fn pop(&self) -> Option<u32> {
        loop {
            let head = self.free_head.load(Ordering::Acquire);
            if head == NONE {
                return None;
            }

            // Read next - safe because we Acquired head, syncs with pusher's Release
            let next = self.slots[head as usize].next.load(Ordering::Relaxed);

            match self.free_head.compare_exchange_weak(
                head,
                next,
                Ordering::Acquire, // Syncs with pusher's Release
                Ordering::Acquire, // On fail, need to see new head
            ) {
                Ok(_) => return Some(head),
                Err(_) => {
                    // Pusher added something newer - retry for hotter item
                    std::hint::spin_loop();
                }
            }
        }
    }

    /// Get reference to value at index.
    ///
    /// # Safety
    ///
    /// Caller must own the slot (have popped it) and slot must contain valid value.
    unsafe fn read_value(&self, idx: u32) -> T {
        unsafe { (*self.slots[idx as usize].value.get()).assume_init_read() }
    }
}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        // Only drop values that are currently in the free list.
        // Values that are "out" (held by Pooled) have been moved
        // out of the slot, and the guard's Drop impl will handle them
        // (either returning to pool, or dropping directly if pool is gone).
        let mut idx = *self.free_head.get_mut();
        while idx != NONE {
            unsafe {
                (*self.slots[idx as usize].value.get()).assume_init_drop();
            }
            idx = *self.slots[idx as usize].next.get_mut();
        }
        // MaybeUninit doesn't drop contents, so Box<[Slot<T>]> will just
        // deallocate memory without double-dropping.
    }
}

// =============================================================================
// Pool - the pool and acquire handle combined
// =============================================================================

/// A bounded pool where one thread acquires and any thread can return.
///
/// Only one `Pool` exists per pool. It cannot be cloned or shared
/// across threads (it is `Send` but not `Sync` or `Clone`).
///
/// When the `Pool` is dropped, outstanding `Pooled` guards
/// will drop their values directly instead of returning them to the pool.
///
/// # Example
///
/// ```
/// use nexus_pool::sync::Pool;
///
/// let acquirer = Pool::new(
///     100,
///     || Vec::<u8>::with_capacity(1024),
///     |v| v.clear(),
/// );
///
/// // Acquirer thread
/// let mut buf = acquirer.try_acquire().unwrap();
/// buf.extend_from_slice(b"hello");
///
/// // Can send buf to another thread
/// std::thread::spawn(move || {
///     println!("{:?}", &*buf);
///     // buf returns to pool on drop
/// }).join().unwrap();
/// ```
pub struct Pool<T> {
    inner: Arc<Inner<T>>,
}

// Pool is Send (can be moved to another thread) but not Sync (not shared)
// Not Clone - only one acquirer exists
// Safety: Inner uses atomics for the free list and values are only accessed
// when a slot is owned (popped). T: Send ensures values can cross threads.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T: Send> Send for Pool<T> {}

impl<T> Pool<T> {
    /// Creates a pool with `capacity` pre-initialized objects.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Number of objects to pre-allocate
    /// * `init` - Factory function to create each object
    /// * `reset` - Called when object returns to pool (e.g., `Vec::clear`)
    ///
    /// # Panics
    ///
    /// Panics if capacity is zero or exceeds `u32::MAX - 1`.
    pub fn new<I, R>(capacity: usize, mut init: I, reset: R) -> Self
    where
        I: FnMut() -> T,
        R: Fn(&mut T) + Send + Sync + 'static,
    {
        assert!(capacity > 0, "capacity must be non-zero");
        assert!(
            capacity < NONE as usize,
            "capacity must be less than {}",
            NONE
        );

        // Build slots with linked free list: 0 -> 1 -> 2 -> ... -> NONE
        let slots: Box<[Slot<T>]> = (0..capacity)
            .map(|i| Slot {
                value: UnsafeCell::new(MaybeUninit::new(init())),
                next: AtomicU32::new(if i + 1 < capacity {
                    (i + 1) as u32
                } else {
                    NONE
                }),
            })
            .collect();

        Self {
            inner: Arc::new(Inner {
                slots,
                free_head: AtomicU32::new(0), // Head of free list
                reset: Box::new(reset),
            }),
        }
    }

    /// Attempts to acquire an object from the pool.
    ///
    /// Returns `None` if all objects are currently in use.
    pub fn try_acquire(&self) -> Option<Pooled<T>> {
        self.inner.pop().map(|idx| {
            // Take value from slot
            // Safety: we just popped this slot, we own it, it contains valid value
            let value = unsafe { self.inner.read_value(idx) };
            Pooled {
                value: ManuallyDrop::new(value),
                idx,
                inner: Arc::downgrade(&self.inner),
            }
        })
    }

    /// Returns the number of available objects.
    ///
    /// Note: This is a snapshot and may be immediately outdated if other
    /// threads are returning objects concurrently.
    pub fn available(&self) -> usize {
        let mut count = 0;
        let mut idx = self.inner.free_head.load(Ordering::Relaxed);
        while idx != NONE {
            count += 1;
            idx = self.inner.slots[idx as usize].next.load(Ordering::Relaxed);
        }
        count
    }
}

// =============================================================================
// Pooled - RAII guard
// =============================================================================

/// RAII guard that returns the object to the pool on drop.
///
/// This guard can be sent to other threads. When dropped, the object
/// is automatically returned to the pool (if the pool still exists).
pub struct Pooled<T> {
    value: ManuallyDrop<T>,
    idx: u32,
    inner: Weak<Inner<T>>,
}

// Pooled is Send + Sync - can be sent anywhere, dropped from anywhere
// Safety: Pooled owns its value (ManuallyDrop<T>). The Weak<Inner<T>> is only
// used during drop to push the slot back via atomic CAS. T: Send ensures the
// value can cross threads; T: Sync ensures shared references are safe.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T: Send> Send for Pooled<T> {}
unsafe impl<T: Send + Sync> Sync for Pooled<T> {}

impl<T> Deref for Pooled<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for Pooled<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T> Drop for Pooled<T> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            // Pool still alive - return value
            let value = unsafe { ManuallyDrop::take(&mut self.value) };
            inner.push(self.idx, value);
        } else {
            // Pool is gone - just drop the value
            unsafe { ManuallyDrop::drop(&mut self.value) };
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    #[test]
    fn basic_acquire_release() {
        let acquirer = Pool::new(3, || Vec::<u8>::with_capacity(16), |v| v.clear());

        let mut a = acquirer.try_acquire().unwrap();
        a.extend_from_slice(b"hello");
        assert_eq!(&*a, b"hello");

        let _b = acquirer.try_acquire().unwrap();
        let _c = acquirer.try_acquire().unwrap();

        // Pool exhausted
        assert!(acquirer.try_acquire().is_none());

        // Return one
        drop(a);

        // Can acquire again - and it's been cleared
        let d = acquirer.try_acquire().unwrap();
        assert!(d.is_empty());
    }

    #[test]
    fn cross_thread_return() {
        let acquirer = Pool::new(2, || 42u32, |_| {});

        let item = acquirer.try_acquire().unwrap();
        assert_eq!(*item, 42);

        // Send to another thread to drop
        thread::spawn(move || {
            assert_eq!(*item, 42);
            drop(item);
        })
        .join()
        .unwrap();

        // Should be back in pool
        let item2 = acquirer.try_acquire().unwrap();
        assert_eq!(*item2, 42);
    }

    #[test]
    fn acquirer_dropped_first() {
        let item;
        {
            let acquirer = Pool::new(1, || String::from("test"), |s| s.clear());
            item = acquirer.try_acquire().unwrap();
            // acquirer drops here
        }
        // item still valid - we can access it
        assert_eq!(&*item, "test");
        // item drops here - should not panic
    }

    #[test]
    fn reset_called_on_return() {
        let reset_count = Arc::new(AtomicUsize::new(0));
        let reset_count_clone = Arc::clone(&reset_count);

        let acquirer = Pool::new(
            2,
            || 0u32,
            move |_| {
                reset_count_clone.fetch_add(1, Ordering::Relaxed);
            },
        );

        let a = acquirer.try_acquire().unwrap();
        assert_eq!(reset_count.load(Ordering::Relaxed), 0);

        drop(a);
        assert_eq!(reset_count.load(Ordering::Relaxed), 1);

        let b = acquirer.try_acquire().unwrap();
        let c = acquirer.try_acquire().unwrap();
        drop(b);
        drop(c);
        assert_eq!(reset_count.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn lifo_ordering() {
        let acquirer = Pool::new(3, || Vec::<u8>::new(), |v| v.clear());

        let mut a = acquirer.try_acquire().unwrap();
        let mut b = acquirer.try_acquire().unwrap();
        let mut c = acquirer.try_acquire().unwrap();

        a.push(1);
        b.push(2);
        c.push(3);

        // Return in order: a, b, c
        drop(a);
        drop(b);
        drop(c);

        // Should get back in LIFO order: c, b, a
        let x = acquirer.try_acquire().unwrap();
        assert!(x.is_empty()); // reset was called, but this was 'c'

        let y = acquirer.try_acquire().unwrap();
        assert!(y.is_empty()); // this was 'b'

        let z = acquirer.try_acquire().unwrap();
        assert!(z.is_empty()); // this was 'a'
    }

    #[test]
    #[should_panic(expected = "capacity must be non-zero")]
    fn zero_capacity_panics() {
        let _ = Pool::new(0, || (), |_| {});
    }

    // =========================================================================
    // Stress tests
    // =========================================================================

    #[test]
    fn stress_single_thread() {
        let acquirer = Pool::new(100, || Vec::<u8>::with_capacity(64), |v| v.clear());

        for _ in 0..10_000 {
            let mut items: Vec<_> = (0..50).filter_map(|_| acquirer.try_acquire()).collect();

            for item in &mut items {
                item.extend_from_slice(b"data");
            }

            drop(items);
        }

        // All items should be back
        let count = acquirer.available();
        assert_eq!(count, 100);
    }

    #[test]
    fn stress_multi_thread_return() {
        let acquirer = Pool::new(
            100,
            || AtomicUsize::new(0),
            |v| {
                v.store(0, Ordering::Relaxed);
            },
        );

        let returned = Arc::new(AtomicUsize::new(0));

        thread::scope(|s| {
            let (tx, rx) = std::sync::mpsc::channel();
            let returned_clone = Arc::clone(&returned);

            // Single worker thread receives and returns items
            s.spawn(move || {
                while let Ok(item) = rx.recv() {
                    let _item: Pooled<AtomicUsize> = item;
                    returned_clone.fetch_add(1, Ordering::Relaxed);
                    // item drops here, returns to pool
                }
            });

            // Main thread acquires and sends to worker
            let mut sent = 0;
            while sent < 1000 {
                if let Some(item) = acquirer.try_acquire() {
                    tx.send(item).unwrap();
                    sent += 1;
                } else {
                    // Pool exhausted, wait a bit for returns
                    thread::yield_now();
                }
            }
            // tx drops here, worker sees disconnect
        });

        assert_eq!(returned.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn stress_concurrent_return() {
        // Multiple threads returning simultaneously
        let acquirer = Pool::new(1000, || 0u64, |_| {});

        // Acquire all items
        let items: Vec<_> = (0..1000).filter_map(|_| acquirer.try_acquire()).collect();
        assert_eq!(items.len(), 1000);

        // Split items across threads and return concurrently
        let items_per_thread = 250;
        let mut item_chunks: Vec<Vec<_>> = Vec::new();
        let mut iter = items.into_iter();
        for _ in 0..4 {
            item_chunks.push(iter.by_ref().take(items_per_thread).collect());
        }

        thread::scope(|s| {
            for chunk in item_chunks {
                s.spawn(move || {
                    for item in chunk {
                        drop(item);
                    }
                });
            }
        });

        // All items should be back
        let count = acquirer.available();
        assert_eq!(count, 1000);
    }
}
