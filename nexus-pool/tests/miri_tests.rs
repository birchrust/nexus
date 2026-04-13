//! Miri tests for nexus-pool object pools.
//!
//! Run: `cargo +nightly miri test -p nexus-pool --test miri_tests`

use std::cell::Cell;
use std::rc::Rc;

#[test]
fn local_bounded_acquire_release() {
    let pool = nexus_pool::local::BoundedPool::new(
        4,
        || Vec::<u8>::with_capacity(64),
        |v: &mut Vec<u8>| v.clear(),
    );

    assert_eq!(pool.available(), 4);

    // Acquire 3 items and use them.
    let mut a = pool.try_acquire().unwrap();
    let mut b = pool.try_acquire().unwrap();
    let mut c = pool.try_acquire().unwrap();

    a.extend_from_slice(b"aaa");
    b.extend_from_slice(b"bbb");
    c.extend_from_slice(b"ccc");

    assert_eq!(pool.available(), 1);

    // Drop guards -- items return to pool.
    drop(a);
    drop(b);
    drop(c);

    assert_eq!(pool.available(), 4);

    // Re-acquire and verify reset was called (vec should be empty).
    let d = pool.try_acquire().unwrap();
    assert!(d.is_empty(), "reset should have cleared the vec");
}

#[test]
fn local_pool_take_put() {
    let pool =
        nexus_pool::local::Pool::new(|| Vec::<u8>::with_capacity(64), |v: &mut Vec<u8>| v.clear());

    // Take a value (creates via factory since pool is empty).
    let mut buf = pool.take();
    buf.extend_from_slice(b"hello");
    assert_eq!(&buf, b"hello");

    // Put it back -- reset (clear) is called.
    pool.put(buf);
    assert_eq!(pool.available(), 1);

    // Take again -- should get the reset (empty) value.
    let reused = pool.take();
    assert!(reused.is_empty(), "reset should have cleared the vec");
    pool.put(reused);
}

#[test]
fn sync_acquire_release() {
    let pool = nexus_pool::sync::Pool::new(
        4,
        || Vec::<u8>::with_capacity(64),
        |v: &mut Vec<u8>| v.clear(),
    );

    assert_eq!(pool.available(), 4);

    // Acquire 2 items.
    let mut a = pool.try_acquire().unwrap();
    let mut b = pool.try_acquire().unwrap();

    assert_eq!(pool.available(), 2);

    a.extend_from_slice(b"aaa");
    b.extend_from_slice(b"bbb");

    // Drop items -- should return to pool.
    drop(a);
    drop(b);

    assert_eq!(pool.available(), 4);

    // Verify reset was applied.
    let c = pool.try_acquire().unwrap();
    assert!(c.is_empty(), "reset should have cleared the vec");
}

#[test]
fn local_drop_tracker() {
    struct Tracked {
        data: Vec<u8>,
        counter: Rc<Cell<u32>>,
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.counter.set(self.counter.get() + 1);
        }
    }

    // Track how many times drop is called across all values.
    let drop_count = Rc::new(Cell::new(0u32));

    let dc = drop_count.clone();
    let pool = nexus_pool::local::BoundedPool::new(
        4,
        move || Tracked {
            data: Vec::with_capacity(64),
            counter: dc.clone(),
        },
        |t: &mut Tracked| t.data.clear(),
    );

    // Acquire 2 items and hold them while dropping the pool.
    let a = pool.try_acquire().unwrap();
    let b = pool.try_acquire().unwrap();
    // 2 items still in the pool, 2 out.

    assert_eq!(drop_count.get(), 0);

    // Drop the pool -- the 2 items still inside should be dropped.
    drop(pool);

    // The 2 items that were in the pool's internal storage are now dropped.
    assert_eq!(drop_count.get(), 2);

    // Drop the outstanding guards -- since the pool is gone, values are
    // dropped directly (not returned).
    drop(a);
    assert_eq!(drop_count.get(), 3);

    drop(b);
    assert_eq!(drop_count.get(), 4);
}
