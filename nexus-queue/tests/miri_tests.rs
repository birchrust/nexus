//! Miri tests for nexus-queue lock-free ring buffers.
//!
//! Run: `cargo +nightly miri test -p nexus-queue --test miri_tests`

use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Debug)]
struct DropCounter(Rc<Cell<u32>>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn spsc_push_pop() {
    let (tx, rx) = nexus_queue::spsc::ring_buffer::<u32>(4);
    for i in 0..16 {
        tx.push(i).unwrap();
        assert_eq!(rx.pop(), Some(i));
    }
}

#[test]
fn spsc_drop_with_pending() {
    let count = Rc::new(Cell::new(0u32));
    {
        let (tx, rx) = nexus_queue::spsc::ring_buffer::<DropCounter>(4);
        for _ in 0..4 {
            tx.push(DropCounter(Rc::clone(&count))).unwrap();
        }
        drop(rx);
        drop(tx);
    }
    // 4 items pushed, none popped — all must be dropped by the ring buffer.
    assert_eq!(count.get(), 4);
}

#[test]
fn spsc_fill_and_drain() {
    let (tx, rx) = nexus_queue::spsc::ring_buffer::<u32>(4);
    for i in 0..4 {
        tx.push(i).unwrap();
    }
    for i in 0..4 {
        assert_eq!(rx.pop(), Some(i));
    }
    assert_eq!(rx.pop(), None);
}

#[test]
fn mpsc_basic() {
    let (tx, rx) = nexus_queue::mpsc::ring_buffer::<u32>(4);
    for i in 0..4 {
        tx.push(i).unwrap();
    }
    for i in 0..4 {
        assert_eq!(rx.pop(), Some(i));
    }
    assert_eq!(rx.pop(), None);
}

#[test]
fn mpsc_clone_producer() {
    let (tx, rx) = nexus_queue::mpsc::ring_buffer::<u32>(8);
    let tx2 = tx.clone();
    for i in 0..4 {
        tx.push(i).unwrap();
        tx2.push(i + 100).unwrap();
    }
    let mut values = Vec::new();
    while let Some(v) = rx.pop() {
        values.push(v);
    }
    assert_eq!(values.len(), 8);
}

#[test]
fn spmc_basic() {
    let (tx, rx) = nexus_queue::spmc::ring_buffer::<u32>(4);
    for i in 0..4 {
        tx.push(i).unwrap();
    }
    let mut values = Vec::new();
    while let Some(v) = rx.pop() {
        values.push(v);
    }
    assert_eq!(values.len(), 4);
}
