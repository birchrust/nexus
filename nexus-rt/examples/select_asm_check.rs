//! Asm spot-check target for `select!`. Build in release and inspect:
//!
//! ```bash
//! cargo asm --example select_asm_check --release \
//!     'select_asm_check::dispatch_select'
//! ```
//!
//! Confirms that the `select!` expansion compiles to a jump table for
//! a dense enum, identical to the hand-written `match` in
//! `dispatch_handwritten`. Compare both functions to verify codegen
//! parity.

use nexus_rt::{World, WorldBuilder, select};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Cmd {
    A,
    B,
    C,
    D,
    E,
    F,
}

#[inline(never)]
fn ha(_w: &mut World, _c: Cmd) {
    std::hint::black_box(0u32);
}
#[inline(never)]
fn hb(_w: &mut World, _c: Cmd) {
    std::hint::black_box(1u32);
}
#[inline(never)]
fn hc(_w: &mut World, _c: Cmd) {
    std::hint::black_box(2u32);
}
#[inline(never)]
fn hd(_w: &mut World, _c: Cmd) {
    std::hint::black_box(3u32);
}
#[inline(never)]
fn he(_w: &mut World, _c: Cmd) {
    std::hint::black_box(4u32);
}
#[inline(never)]
fn hf(_w: &mut World, _c: Cmd) {
    std::hint::black_box(5u32);
}

// Reference: the hand-written match. cargo-asm will show a jump table.
#[inline(never)]
pub fn dispatch_handwritten(world: &mut World, cmd: Cmd) {
    match cmd {
        Cmd::A => ha(world, cmd),
        Cmd::B => hb(world, cmd),
        Cmd::C => hc(world, cmd),
        Cmd::D => hd(world, cmd),
        Cmd::E => he(world, cmd),
        Cmd::F => hf(world, cmd),
    }
}

// The select!-produced closure, called directly. Bypasses the pipeline
// builder entirely so cargo-asm sees only the dispatch body.
#[inline(never)]
pub fn dispatch_select(world: &mut World, cmd: Cmd) {
    // Note: we need a registry only for `resolve_step` calls in the
    // expansion. Since our handlers take no resources, an empty
    // registry suffices. We construct the dispatcher once per call
    // here purely to expose codegen — in real use it'd be built once
    // at startup.
    let wb = WorldBuilder::new();
    let dummy = wb.build();
    let reg = dummy.registry();
    let mut dispatch = select! {
        reg,
        Cmd::A => ha,
        Cmd::B => hb,
        Cmd::C => hc,
        Cmd::D => hd,
        Cmd::E => he,
        Cmd::F => hf,
    };
    dispatch(world, cmd);
}

fn main() {
    let mut world = WorldBuilder::new().build();
    for cmd in [Cmd::A, Cmd::B, Cmd::C, Cmd::D, Cmd::E, Cmd::F] {
        dispatch_handwritten(&mut world, cmd);
        dispatch_select(&mut world, cmd);
    }
}
