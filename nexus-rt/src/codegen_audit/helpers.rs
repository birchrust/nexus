//! Shared types, resources, and step functions for codegen audit.

use crate::{Res, ResMut, World, WorldBuilder};

// ── Resource types ───────────────────────────────────────────────

/// Simple mutable counter.
pub struct ResA(pub u64);
/// Secondary resource for multi-param tests.
pub struct ResB(pub u32);
/// Third resource for high-arity tests.
pub struct ResC(pub u16);
/// Fourth resource.
pub struct ResD(pub u8);
/// Fifth resource (float).
pub struct ResE(pub f64);
/// Sixth resource for high-arity tests.
pub struct ResF(pub u64);
/// Seventh resource for high-arity tests.
pub struct ResG(pub u32);
/// Eighth resource for high-arity tests.
pub struct ResH(pub u16);

// ── World factory ────────────────────────────────────────────────

/// Build a World pre-populated with all audit resources.
pub fn make_world() -> World {
    let mut wb = WorldBuilder::new();
    wb.register(ResA(100));
    wb.register(ResB(50));
    wb.register(ResC(25));
    wb.register(ResD(10));
    wb.register(ResE(3.14));
    wb.register(ResF(200));
    wb.register(ResG(75));
    wb.register(ResH(12));
    wb.build()
}

// ═════════════════════════════════════════════════════════════════
// Pipeline steps (by-value input)
// ═════════════════════════════════════════════════════════════════

// -- Arity 0 — pure transforms -----------------------------------

pub fn add_one(x: u64) -> u64 { x.wrapping_add(1) }
pub fn double(x: u64) -> u64 { x.wrapping_mul(2) }
pub fn add_three(x: u64) -> u64 { x.wrapping_add(3) }
pub fn square(x: u64) -> u64 { x.wrapping_mul(x) }
pub fn sub_ten(x: u64) -> u64 { x.wrapping_sub(10) }
pub fn shr_one(x: u64) -> u64 { x >> 1 }
pub fn xor_mask(x: u64) -> u64 { x ^ 0xDEAD_BEEF }
pub fn add_seven(x: u64) -> u64 { x.wrapping_add(7) }
pub fn triple(x: u64) -> u64 { x.wrapping_mul(3) }
pub fn add_forty_two(x: u64) -> u64 { x.wrapping_add(42) }

// -- Arity 1 — one Res/ResMut ------------------------------------

pub fn add_res_a(a: Res<ResA>, x: u64) -> u64 { x.wrapping_add(a.0) }
pub fn mul_res_b(b: Res<ResB>, x: u64) -> u64 { x.wrapping_mul(b.0 as u64) }
pub fn write_res_a(mut a: ResMut<ResA>, x: u64) -> u64 {
    a.0 = a.0.wrapping_add(x);
    x
}

// -- Arity 2 ------------------------------------------------------

pub fn add_both(a: Res<ResA>, b: Res<ResB>, x: u64) -> u64 {
    x.wrapping_add(a.0).wrapping_add(b.0 as u64)
}

// -- Arity 3 ------------------------------------------------------

pub fn three_params(a: Res<ResA>, b: Res<ResB>, c: Res<ResC>, x: u64) -> u64 {
    x.wrapping_add(a.0)
        .wrapping_add(b.0 as u64)
        .wrapping_add(c.0 as u64)
}

// -- Arity 4 ------------------------------------------------------

pub fn four_params(
    a: Res<ResA>, b: Res<ResB>, c: Res<ResC>, d: Res<ResD>, x: u64,
) -> u64 {
    x.wrapping_add(a.0)
        .wrapping_add(b.0 as u64)
        .wrapping_add(c.0 as u64)
        .wrapping_add(d.0 as u64)
}

// -- Arity 5 ------------------------------------------------------

pub fn five_params(
    a: Res<ResA>, b: Res<ResB>, c: Res<ResC>, d: Res<ResD>, e: Res<ResE>, x: u64,
) -> u64 {
    x.wrapping_add(a.0)
        .wrapping_add(b.0 as u64)
        .wrapping_add(c.0 as u64)
        .wrapping_add(d.0 as u64)
        .wrapping_add(e.0 as u64)
}

// -- Arity 7 (8 params total with event) --------------------------

pub fn eight_params(
    a: Res<ResA>, b: Res<ResB>, c: Res<ResC>, d: Res<ResD>,
    e: Res<ResE>, f: Res<ResF>, g: Res<ResG>, x: u64,
) -> u64 {
    x.wrapping_add(a.0)
        .wrapping_add(b.0 as u64)
        .wrapping_add(c.0 as u64)
        .wrapping_add(d.0 as u64)
        .wrapping_add(e.0 as u64)
        .wrapping_add(f.0)
        .wrapping_add(g.0 as u64)
}

// -- Option-producing steps ---------------------------------------

pub fn maybe_positive(x: u64) -> Option<u64> {
    if x > 0 { Some(x) } else { None }
}

pub fn checked_double(x: u64) -> Option<u64> { x.checked_mul(2) }

// -- Result-producing steps ---------------------------------------

pub fn try_parse(x: u64) -> Result<u64, u32> {
    if x < 10_000 { Ok(x.wrapping_mul(2)) } else { Err(x as u32) }
}

pub fn validate(a: Res<ResA>, x: u64) -> Result<u64, u32> {
    if x <= a.0 { Ok(x) } else { Err(x as u32) }
}

// -- Bool-producing steps -----------------------------------------

pub fn is_even(x: u64) -> bool { x & 1 == 0 }

// -- Tuple-producing steps ----------------------------------------

pub fn split_u64(x: u64) -> (u32, u32) {
    (x as u32, (x >> 32) as u32)
}

pub fn split_3(x: u64) -> (u32, u16, u8) {
    (x as u32, (x >> 32) as u16, (x >> 48) as u8)
}

pub fn split_4(x: u64) -> (u32, u16, u8, u8) {
    (x as u32, (x >> 32) as u16, (x >> 48) as u8, (x >> 56) as u8)
}

pub fn split_5(x: u64) -> (u32, u16, u8, u8, u8) {
    (x as u32, (x >> 32) as u16, (x >> 48) as u8, (x >> 52) as u8, (x >> 56) as u8)
}

// -- Sinks (return ()) --------------------------------------------

pub fn consume_val(mut a: ResMut<ResA>, x: u64) { a.0 = a.0.wrapping_add(x); }
pub fn consume_unit(_x: ()) {}

// ═════════════════════════════════════════════════════════════════
// DAG steps (by-reference input)
// ═════════════════════════════════════════════════════════════════

pub fn ref_add_one(x: &u64) -> u64 { x.wrapping_add(1) }
pub fn ref_double(x: &u64) -> u64 { x.wrapping_mul(2) }
pub fn ref_add_three(x: &u64) -> u64 { x.wrapping_add(3) }
pub fn ref_square(x: &u64) -> u64 { x.wrapping_mul(*x) }
pub fn ref_sub_ten(x: &u64) -> u64 { x.wrapping_sub(10) }
pub fn ref_shr_one(x: &u64) -> u64 { x >> 1 }
pub fn ref_xor_mask(x: &u64) -> u64 { x ^ 0xDEAD_BEEF }
pub fn ref_add_seven(x: &u64) -> u64 { x.wrapping_add(7) }
pub fn ref_triple(x: &u64) -> u64 { x.wrapping_mul(3) }
pub fn ref_add_forty_two(x: &u64) -> u64 { x.wrapping_add(42) }

pub fn ref_add_res_a(a: Res<ResA>, x: &u64) -> u64 { x.wrapping_add(a.0) }
pub fn ref_mul_res_b(b: Res<ResB>, x: &u64) -> u64 { x.wrapping_mul(b.0 as u64) }
pub fn ref_write_res_a(mut a: ResMut<ResA>, x: &u64) -> u64 {
    a.0 = a.0.wrapping_add(*x);
    *x
}

pub fn ref_add_both(a: Res<ResA>, b: Res<ResB>, x: &u64) -> u64 {
    x.wrapping_add(a.0).wrapping_add(b.0 as u64)
}

pub fn ref_three_params(a: Res<ResA>, b: Res<ResB>, c: Res<ResC>, x: &u64) -> u64 {
    x.wrapping_add(a.0)
        .wrapping_add(b.0 as u64)
        .wrapping_add(c.0 as u64)
}

pub fn ref_maybe_positive(x: &u64) -> Option<u64> {
    if *x > 0 { Some(*x) } else { None }
}

pub fn ref_try_parse(x: &u64) -> Result<u64, u32> {
    if *x < 10_000 { Ok(x.wrapping_mul(2)) } else { Err(*x as u32) }
}

pub fn ref_is_even(x: &u64) -> bool { x & 1 == 0 }

pub fn ref_split_u64(x: &u64) -> (u32, u32) {
    (*x as u32, (x >> 32) as u32)
}

pub fn ref_split_3(x: &u64) -> (u32, u16, u8) {
    (*x as u32, (x >> 32) as u16, (x >> 48) as u8)
}

pub fn ref_five_params(
    a: Res<ResA>, b: Res<ResB>, c: Res<ResC>, d: Res<ResD>, e: Res<ResE>, x: &u64,
) -> u64 {
    x.wrapping_add(a.0)
        .wrapping_add(b.0 as u64)
        .wrapping_add(c.0 as u64)
        .wrapping_add(d.0 as u64)
        .wrapping_add(e.0 as u64)
}

pub fn ref_split_5(x: &u64) -> (u32, u16, u8, u8, u8) {
    (*x as u32, (x >> 32) as u16, (x >> 48) as u8, (x >> 52) as u8, (x >> 56) as u8)
}

pub fn ref_consume(mut a: ResMut<ResA>, x: &u64) { a.0 = a.0.wrapping_add(*x); }
pub fn ref_consume_unit(_x: &()) {}

// ═════════════════════════════════════════════════════════════════
// Merge steps (multi-reference input)
// ═════════════════════════════════════════════════════════════════

pub fn merge_add(a: &u64, b: &u64) -> u64 { a.wrapping_add(*b) }
pub fn merge_mul(a: &u64, b: &u64) -> u64 { a.wrapping_mul(*b) }

pub fn merge_3(a: &u64, b: &u64, c: &u64) -> u64 {
    a.wrapping_add(*b).wrapping_add(*c)
}

pub fn merge_4(a: &u64, b: &u64, c: &u64, d: &u64) -> u64 {
    a.wrapping_add(*b).wrapping_add(*c).wrapping_add(*d)
}

pub fn merge_consume(mut w: ResMut<ResA>, a: &u64, b: &u64) {
    w.0 = a.wrapping_add(*b);
}

pub fn merge_3_consume(mut w: ResMut<ResA>, a: &u64, b: &u64, c: &u64) {
    w.0 = a.wrapping_add(*b).wrapping_add(*c);
}

// ═════════════════════════════════════════════════════════════════
// Splat steps (tuple elements as individual args)
// ═════════════════════════════════════════════════════════════════

pub fn splat_add(a: u32, b: u32) -> u64 { a as u64 + b as u64 }

pub fn splat_3(a: u32, b: u16, c: u8) -> u64 {
    a as u64 + b as u64 + c as u64
}

pub fn splat_4(a: u32, b: u16, c: u8, d: u8) -> u64 {
    a as u64 + b as u64 + c as u64 + d as u64
}

pub fn splat_5(a: u32, b: u16, c: u8, d: u8, e: u8) -> u64 {
    a as u64 + b as u64 + c as u64 + d as u64 + e as u64
}

// DAG splat (by reference)

pub fn ref_splat_add(a: &u32, b: &u32) -> u64 { *a as u64 + *b as u64 }

pub fn ref_splat_3(a: &u32, b: &u16, c: &u8) -> u64 {
    *a as u64 + *b as u64 + *c as u64
}

pub fn ref_splat_5(a: &u32, b: &u16, c: &u8, d: &u8, e: &u8) -> u64 {
    *a as u64 + *b as u64 + *c as u64 + *d as u64 + *e as u64
}
