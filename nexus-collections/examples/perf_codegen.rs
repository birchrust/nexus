//! Codegen inspection: #[inline(never)] wrappers for all critical methods.
//!
//! Build with:
//!   cargo rustc --package nexus-collections --example perf_codegen --release -- --emit asm -C "llvm-args=-x86-asm-syntax=intel"
//!
//! Then grep for `do_` functions in the .s file.

use nexus_collections::heap::{Heap as HeapTrait, HeapNode};
use nexus_collections::list::{List as ListTrait, ListNode};
use std::hint::black_box;

// --- Heap allocator ---
mod hpq {
    nexus_collections::heap_allocator!(u64, bounded);
}

// --- List allocator ---
mod lq {
    nexus_collections::list_allocator!(u64, bounded);
}

// =============================================================================
// Heap wrappers
// =============================================================================

#[inline(never)]
fn heap_push(heap: &mut hpq::Heap, h: &hpq::Handle) {
    heap.push(h);
}

#[inline(never)]
fn heap_pop(heap: &mut hpq::Heap) -> Option<hpq::Handle> {
    heap.pop()
}

#[inline(never)]
fn heap_unlink(heap: &mut hpq::Heap, h: &hpq::Handle) {
    heap.unlink(h);
}

#[inline(never)]
fn heap_peek(heap: &hpq::Heap) -> Option<&HeapNode<u64>> {
    heap.peek()
}

// =============================================================================
// List wrappers
// =============================================================================

#[inline(never)]
fn list_link_back(list: &mut lq::List, h: &lq::Handle) {
    list.link_back(h);
}

#[inline(never)]
fn list_link_front(list: &mut lq::List, h: &lq::Handle) {
    list.link_front(h);
}

#[inline(never)]
fn list_unlink(list: &mut lq::List, h: &lq::Handle) {
    list.unlink(h);
}

#[inline(never)]
fn list_pop_front(list: &mut lq::List) -> Option<lq::Handle> {
    list.pop_front()
}

#[inline(never)]
fn list_pop_back(list: &mut lq::List) -> Option<lq::Handle> {
    list.pop_back()
}

#[inline(never)]
fn list_move_to_front(list: &mut lq::List, h: &lq::Handle) {
    list.move_to_front(h);
}

#[inline(never)]
fn list_move_to_back(list: &mut lq::List, h: &lq::Handle) {
    list.move_to_back(h);
}

fn main() {
    // Heap
    hpq::Allocator::builder().capacity(100).build().unwrap();
    let mut heap = hpq::Heap::new();
    let h1 = hpq::create_node(10).unwrap();
    let h2 = hpq::create_node(5).unwrap();
    let h3 = hpq::create_node(20).unwrap();

    heap_push(&mut heap, &h1);
    heap_push(&mut heap, &h2);
    heap_push(&mut heap, &h3);
    black_box(heap_peek(&heap));
    heap_unlink(&mut heap, &h3);
    while let Some(p) = heap_pop(&mut heap) {
        black_box(p.data());
    }

    // List
    lq::Allocator::builder().capacity(100).build().unwrap();
    let mut list = lq::List::new();
    let l1 = lq::create_node(10).unwrap();
    let l2 = lq::create_node(5).unwrap();
    let l3 = lq::create_node(20).unwrap();
    let l4 = lq::create_node(30).unwrap();

    list_link_back(&mut list, &l1);
    list_link_back(&mut list, &l2);
    list_link_front(&mut list, &l3);
    list_link_back(&mut list, &l4);
    list_move_to_front(&mut list, &l2);
    list_move_to_back(&mut list, &l3);
    list_unlink(&mut list, &l4);
    while let Some(p) = list_pop_front(&mut list) {
        black_box(&*p);
    }
    // Use pop_back once
    list_link_back(&mut list, &l1);
    black_box(list_pop_back(&mut list));
}
