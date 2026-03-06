//! Pipeline codegen audit cases.
//!
//! Categories 1-12 from the assembly audit plan.

#![allow(clippy::type_complexity)]
#![allow(unused_variables)]

use crate::dag::DagArmStart;
use crate::pipeline::{PipelineStart, resolve_step};
use crate::{Broadcast, IntoHandler, Local, Res, ResMut, Sequence, World, fan_out};
use super::helpers::*;

// ═══════════════════════════════════════════════════════════════════
// 1. Linear chains
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_linear_1(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new().then(add_one, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_linear_2(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_linear_3(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(add_three, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_linear_5(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(add_three, &reg)
        .then(square, &reg)
        .then(sub_ten, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_linear_10(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(add_three, &reg)
        .then(square, &reg)
        .then(sub_ten, &reg)
        .then(shr_one, &reg)
        .then(xor_mask, &reg)
        .then(add_seven, &reg)
        .then(triple, &reg)
        .then(add_forty_two, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_linear_20(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(add_three, &reg)
        .then(square, &reg)
        .then(sub_ten, &reg)
        .then(shr_one, &reg)
        .then(xor_mask, &reg)
        .then(add_seven, &reg)
        .then(triple, &reg)
        .then(add_forty_two, &reg)
        .then(add_one, &reg)
        .then(double, &reg)
        .then(add_three, &reg)
        .then(square, &reg)
        .then(sub_ten, &reg)
        .then(shr_one, &reg)
        .then(xor_mask, &reg)
        .then(add_seven, &reg)
        .then(triple, &reg)
        .then(add_forty_two, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_linear_0_params(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(add_three, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_linear_mixed_arity(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(add_res_a, &reg)
        .then(write_res_a, &reg)
        .then(add_both, &reg)
        .then(three_params, &reg);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 2. Guard, filter, dedup
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_guard_basic(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .guard(|_w, x| *x > 10)
        .inspect(|_w, _x| {});
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_guard_then(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .guard(|_w, x| *x > 10)
        .map(double, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_filter_basic(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .guard(|_w, x| *x > 10)
        .filter(|_w, x| *x < 1000)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_dedup(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dedup()
        .inspect(|_w, _x| {});
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_dedup_chain(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dedup()
        .map(double, &reg)
        .unwrap_or_else(|_w| 0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_guard_large_type(world: &mut World, input: [u8; 256]) -> Option<[u8; 256]> {
    let mut p = PipelineStart::<[u8; 256]>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| x[0] > 0);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 3a. Individual Option combinators
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_opt_map(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .map(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_and_then(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .and_then(checked_double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_filter(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .filter(|_w, x| *x < 1000);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_inspect(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .inspect(|_w, _x| {});
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_on_none(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .on_none(|_w| {});
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_ok_or(world: &mut World, input: u64) -> Result<u64, u32> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .ok_or(42u32);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_ok_or_else(world: &mut World, input: u64) -> Result<u64, u32> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .ok_or_else(|_w| 42u32);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_unwrap_or(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_unwrap_or_else(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .unwrap_or_else(|_w| 0);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 3b. Option combinator chains
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_opt_map_filter(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .map(double, &reg)
        .filter(|_w, x| *x < 1000)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_map_and_then(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .map(double, &reg)
        .and_then(checked_double, &reg)
        .unwrap_or_else(|_w| 0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_filter_inspect_unwrap(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .filter(|_w, x| *x < 1000)
        .inspect(|_w, _x| {})
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_triple_filter(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .filter(|_w, x| *x < 10000)
        .filter(|_w, x| *x > 5)
        .filter(|_w, x| x & 1 == 0)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_map_5x(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .map(add_one, &reg)
        .map(double, &reg)
        .map(add_three, &reg)
        .map(square, &reg)
        .map(sub_ten, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

// -- Early termination stress: guard → long dead chain on None path --

#[inline(never)]
pub fn pipe_opt_guard_skip_10(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    // The guard at step 1 can return None. The 10 maps after it should
    // all be dead code on that path — does the compiler branch once and
    // skip everything?
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 100)
        .map(add_one, &reg)
        .map(double, &reg)
        .map(add_three, &reg)
        .map(square, &reg)
        .map(sub_ten, &reg)
        .map(shr_one, &reg)
        .map(xor_mask, &reg)
        .map(add_seven, &reg)
        .map(triple, &reg)
        .map(add_forty_two, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_opt_filter_skip_chain(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    // Guard → filter at step 2 → 8 more maps. If filter returns None,
    // all 8 maps should be skipped.
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .filter(|_w, x| *x < 50)
        .map(add_one, &reg)
        .map(double, &reg)
        .map(add_three, &reg)
        .map(square, &reg)
        .map(sub_ten, &reg)
        .map(shr_one, &reg)
        .map(xor_mask, &reg)
        .map(add_seven, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 4a. Individual Result combinators
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_res_map(world: &mut World, input: u64) -> Result<u64, u32> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .map(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_and_then(world: &mut World, input: u64) -> Result<u64, u32> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .and_then(try_parse, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_catch(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .catch(log_error, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_map_err(world: &mut World, input: u64) -> Result<u64, u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .map_err(|_w, e| e as u64);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_or_else(world: &mut World, input: u64) -> Result<u64, u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .or_else(|_w, e| Err(e as u64));
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_inspect(world: &mut World, input: u64) -> Result<u64, u32> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .inspect(|_w, _x| {});
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_inspect_err(world: &mut World, input: u64) -> Result<u64, u32> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .inspect_err(|_w, _e| {});
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_ok(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .ok();
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_unwrap_or(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_unwrap_or_else(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .unwrap_or_else(|_w, e| e as u64);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 4b. Result combinator chains
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_res_map_and_then(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .map(double, &reg)
        .and_then(try_parse, &reg)
        .unwrap_or_else(|_w, e| e as u64);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_map_err_or_else(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .map_err(|_w, e| e as u64)
        .or_else(|_w, e| Ok::<u64, u64>(e))
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_inspect_both(world: &mut World, input: u64) -> Option<u64> {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .inspect(|_w, _v| {})
        .inspect_err(|_w, _e| {})
        .ok();
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_catch_then_option(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .catch(log_error, &reg)
        .map(double, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_map_5x(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .map(add_one, &reg)
        .map(double, &reg)
        .map(add_three, &reg)
        .map(square, &reg)
        .map(sub_ten, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

// -- Early termination stress: Err → long dead chain on Err path --

#[inline(never)]
pub fn pipe_res_err_skip_10(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    // try_parse returns Err for input >= 10_000. The 10 maps after it
    // should all be skipped on the Err path — single branch or per-step?
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .map(add_one, &reg)
        .map(double, &reg)
        .map(add_three, &reg)
        .map(square, &reg)
        .map(sub_ten, &reg)
        .map(shr_one, &reg)
        .map(xor_mask, &reg)
        .map(add_seven, &reg)
        .map(triple, &reg)
        .map(add_forty_two, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_res_catch_skip_chain(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    // try_parse → catch converts Err → None. Then 8 option maps should
    // all be dead code if the original result was Err.

    fn log_error(_err: u32) {}

    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .catch(log_error, &reg)
        .map(add_one, &reg)
        .map(double, &reg)
        .map(add_three, &reg)
        .map(square, &reg)
        .map(sub_ten, &reg)
        .map(shr_one, &reg)
        .map(xor_mask, &reg)
        .map(add_seven, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 5. Type transitions
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_trans_guard_unwrap(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .guard(|_w, x| *x > 10)
        .unwrap_or(0)
        .then(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_trans_guard_ok_or_unwrap(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .ok_or(0u32)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_trans_result_ok_unwrap(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .ok()
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_trans_result_catch_unwrap(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    let mut p = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .catch(log_error, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_trans_guard_ok_or_catch_unwrap(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .ok_or(0u32)
        .catch(log_error, &reg)
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_trans_full_lifecycle(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .guard(|_w, x| *x > 0)
        .ok_or(0u32)
        .map(double, &reg)
        .catch(log_error, &reg)
        .unwrap_or(0)
        .then(add_three, &reg);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 6. Branching (route, switch)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_route_basic(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    let on_true = PipelineStart::<u64>::new().then(double, &reg);
    let on_false = PipelineStart::<u64>::new().then(add_one, &reg);

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .route(|_w, x| *x > 100, on_true, on_false)
        .then(add_three, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_route_nested_2(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    let arm_a = PipelineStart::<u64>::new().then(double, &reg);
    let arm_b = PipelineStart::<u64>::new().then(triple, &reg);
    let arm_c = PipelineStart::<u64>::new().then(add_one, &reg);

    let inner_false = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .route(|_w, x| *x > 50, arm_b, arm_c);

    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .route(|_w, x| *x > 100, arm_a, inner_false);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_route_heavy_arms(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    let arm_t = PipelineStart::<u64>::new()
        .then(double, &reg)
        .then(add_three, &reg)
        .then(square, &reg)
        .then(sub_ten, &reg)
        .then(shr_one, &reg);

    let arm_f = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(triple, &reg)
        .then(xor_mask, &reg)
        .then(add_seven, &reg)
        .then(add_forty_two, &reg);

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .route(|_w, x| *x > 100, arm_t, arm_f);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_switch_basic(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .switch(|_w, x| if x > 100 { x.wrapping_mul(2) } else { x.wrapping_add(1) });
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_switch_3way(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| match x % 3 {
            0 => x.wrapping_mul(2),
            1 => x.wrapping_add(10),
            _ => x.wrapping_sub(5),
        });
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_switch_resolve_step(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut arm_big = resolve_step(double, &reg);
    let mut arm_small = resolve_step(add_one, &reg);

    let mut p = PipelineStart::<u64>::new()
        .switch(move |world, x| {
            if x > 100 {
                arm_big(world, x)
            } else {
                arm_small(world, x)
            }
        });
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_start_switch(world: &mut World, input: u64) -> u64 {
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x.wrapping_mul(2).wrapping_add(1));
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 7. Splat
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_splat2_start(world: &mut World, input: (u32, u32)) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<(u32, u32)>::new()
        .splat()
        .then(splat_add, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_splat2_mid(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(split_u64, &reg)
        .splat()
        .then(splat_add, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_splat3(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(split_3, &reg)
        .splat()
        .then(splat_3, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_splat4(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(split_4, &reg)
        .splat()
        .then(splat_4, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_splat5(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(split_5, &reg)
        .splat()
        .then(splat_5, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_splat_with_params(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn splat_with_res(a: Res<ResA>, x: u32, y: u32) -> u64 {
        x as u64 + y as u64 + a.0
    }

    let mut p = PipelineStart::<u64>::new()
        .then(split_u64, &reg)
        .splat()
        .then(splat_with_res, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_splat_then_guard(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(split_u64, &reg)
        .splat()
        .then(splat_add, &reg)
        .guard(|_w, x| *x > 10)
        .unwrap_or(0);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 8. Bool combinators
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_bool_not(world: &mut World, input: u64) -> bool {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(is_even, &reg)
        .not();
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_bool_and(world: &mut World, input: u64) -> bool {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(is_even, &reg)
        .and(|_w| true);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_bool_or(world: &mut World, input: u64) -> bool {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(is_even, &reg)
        .or(|_w| false);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_bool_xor(world: &mut World, input: u64) -> bool {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(is_even, &reg)
        .xor(|_w| true);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_bool_chain(world: &mut World, input: u64) -> bool {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(is_even, &reg)
        .and(|_w| true)
        .or(|_w| false)
        .not()
        .xor(|_w| true);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_bool_guard_integration(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(is_even, &reg)
        .and(|_w| true)
        .guard(|_w, &b| b)
        .unwrap_or(false)
        .switch(|_w, b| if b { 1u64 } else { 0u64 });
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 9. Clone transitions
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_cloned_copy_type(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn take_ref(x: u64) -> &'static u64 {
        // Leak to get 'static for the audit — the point is the cloned() codegen.
        Box::leak(Box::new(x))
    }

    let mut p = PipelineStart::<u64>::new()
        .then(take_ref, &reg)
        .cloned()
        .then(double, &reg);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 10. Side effects (tap, tee)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_tap_basic(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .tap(|_w, _x| {})
        .then(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_tap_multiple(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .tap(|_w, _x| {})
        .then(add_one, &reg)
        .tap(|_w, _x| {})
        .then(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_tee_basic(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    let side = DagArmStart::<u64>::new()
        .then(ref_consume, &reg);

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .tee(side)
        .then(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_tee_heavy(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    let side = DagArmStart::<u64>::new()
        .then(ref_add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_add_three, &reg)
        .then(ref_square, &reg)
        .then(ref_consume, &reg);

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .tee(side)
        .then(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_tap_with_world(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .tap(|w, x| {
            // Force a World access in the tap closure.
            let _ = w.resource::<ResA>().0.wrapping_add(*x);
        })
        .then(double, &reg);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 11. Dispatch & fan-out
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_dispatch_handler(world: &mut World, input: u64) {
    let reg = world.registry();
    let handler = consume_val.into_handler(&reg);
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dispatch(handler);
    p.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 12. World interaction (param resolution codegen)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn pipe_world_res(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_res_a, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_res_mut(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(write_res_a, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_res_res_mut(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_res_a, &reg)
        .then(write_res_a, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_3_params(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(three_params, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_mixed_chain(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(add_res_a, &reg)
        .then(write_res_a, &reg)
        .then(double, &reg)
        .then(add_both, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_change_detection(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn check_changed(a: Res<ResA>, x: u64) -> u64 {
        if a.is_changed() { x.wrapping_mul(2) } else { x }
    }

    let mut p = PipelineStart::<u64>::new()
        .then(write_res_a, &reg)
        .then(check_changed, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_res_mut_stamp(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn stamp_and_pass(mut a: ResMut<ResA>, x: u64) -> u64 {
        *a = ResA(x);
        x
    }

    let mut p = PipelineStart::<u64>::new()
        .then(stamp_and_pass, &reg);
    p.run(world, input)
}

// ---- Remaining section 3b gap: 3.13 ----

#[inline(never)]
pub fn pipe_opt_ok_or_chain(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    // guard → ok_or → Result combinators → unwrap
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .ok_or(0u32)
        .map(double, &reg)
        .and_then(try_parse, &reg)
        .unwrap_or_else(|_w, e| e as u64);
    p.run(world, input)
}

// ---- Remaining section 5 gaps: 5.6, 5.7 ----

#[inline(never)]
pub fn pipe_trans_nested_option(world: &mut World, input: u64) -> Option<Option<u64>> {
    let reg = world.registry();
    // maybe_positive → Option<u64>, then map(checked_double) → Option<Option<u64>>
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .map(checked_double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_trans_result_in_option(world: &mut World, input: u64) -> Option<Result<u64, u32>> {
    let reg = world.registry();
    // maybe_positive → Option<u64>, then map(try_parse) → Option<Result<u64, u32>>
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .map(try_parse, &reg);
    p.run(world, input)
}

// ---- Remaining section 6 gaps: 6.3 ----

#[inline(never)]
pub fn pipe_route_nested_3(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    let arm_a = PipelineStart::<u64>::new().then(double, &reg);
    let arm_b = PipelineStart::<u64>::new().then(triple, &reg);
    let arm_c = PipelineStart::<u64>::new().then(add_one, &reg);
    let arm_d = PipelineStart::<u64>::new().then(add_seven, &reg);

    // Inner-most route: 3rd level
    let inner2 = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .route(|_w, x| *x > 25, arm_c, arm_d);

    // 2nd level
    let inner1 = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .route(|_w, x| *x > 50, arm_b, inner2);

    // Top level — 4-way via nesting
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .route(|_w, x| *x > 100, arm_a, inner1);
    p.run(world, input)
}

// ---- Remaining section 7 gap: 7.8 ----

#[inline(never)]
pub fn pipe_splat_large_types(world: &mut World, input: u64) -> u64 {
    fn make_pair(x: u64) -> ([u8; 64], [u8; 64]) {
        let mut a = [0u8; 64];
        let mut b = [0u8; 64];
        a[0] = x as u8;
        b[0] = (x >> 8) as u8;
        (a, b)
    }

    fn combine_pair(a: [u8; 64], b: [u8; 64]) -> u64 {
        a[0] as u64 + b[0] as u64
    }

    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(make_pair, &reg)
        .splat()
        .then(combine_pair, &reg);
    p.run(world, input)
}

// ---- Remaining section 9 gaps: 9.1, 9.3, 9.4, 9.5 ----

#[inline(never)]
pub fn pipe_cloned_bare(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn take_ref(x: u64) -> &'static u64 {
        Box::leak(Box::new(x))
    }

    // &u64 → u64 via cloned() — should be a single load
    let mut p = PipelineStart::<u64>::new()
        .then(take_ref, &reg)
        .cloned()
        .then(double, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_cloned_option(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn maybe_ref(x: u64) -> Option<&'static u64> {
        if x > 0 { Some(Box::leak(Box::new(x))) } else { None }
    }

    // Option<&u64> → Option<u64> via cloned()
    let mut p = PipelineStart::<u64>::new()
        .then(maybe_ref, &reg)
        .cloned()
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_cloned_result(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn try_ref(x: u64) -> Result<&'static u64, u32> {
        if x < 10_000 { Ok(Box::leak(Box::new(x))) } else { Err(x as u32) }
    }

    // Result<&u64, u32> → Result<u64, u32> via cloned()
    let mut p = PipelineStart::<u64>::new()
        .then(try_ref, &reg)
        .cloned()
        .unwrap_or(0);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_cloned_large_type(world: &mut World, input: u64) -> [u8; 256] {
    let reg = world.registry();

    fn make_ref(x: u64) -> &'static [u8; 256] {
        let mut arr = [0u8; 256];
        arr[0] = x as u8;
        Box::leak(Box::new(arr))
    }

    // &[u8; 256] → [u8; 256] via cloned() — should be memcpy
    let mut p = PipelineStart::<u64>::new()
        .then(make_ref, &reg)
        .cloned();
    p.run(world, input)
}

// ---- Remaining section 11 gaps: 11.2-11.7 ----

#[inline(never)]
pub fn pipe_dispatch_fanout2(world: &mut World, input: u64) {
    let reg = world.registry();

    fn sink_a(mut a: ResMut<ResA>, x: &u64) { a.0 = a.0.wrapping_add(*x); }
    fn sink_b(mut a: ResMut<ResB>, x: &u64) { a.0 = a.0.wrapping_add(*x as u32); }

    let h1 = sink_a.into_handler(&reg);
    let h2 = sink_b.into_handler(&reg);

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dispatch(fan_out!(h1, h2));
    p.run(world, input);
}

#[inline(never)]
pub fn pipe_dispatch_fanout4(world: &mut World, input: u64) {
    let reg = world.registry();

    fn sink_a(mut a: ResMut<ResA>, x: &u64) { a.0 = a.0.wrapping_add(*x); }
    fn sink_b(mut a: ResMut<ResB>, x: &u64) { a.0 = a.0.wrapping_add(*x as u32); }
    fn sink_c(mut a: ResMut<ResC>, x: &u64) { a.0 = a.0.wrapping_add(*x as u16); }
    fn sink_d(mut a: ResMut<ResD>, x: &u64) { a.0 = a.0.wrapping_add(*x as u8); }

    let h1 = sink_a.into_handler(&reg);
    let h2 = sink_b.into_handler(&reg);
    let h3 = sink_c.into_handler(&reg);
    let h4 = sink_d.into_handler(&reg);

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dispatch(fan_out!(h1, h2, h3, h4));
    p.run(world, input);
}

#[inline(never)]
pub fn pipe_dispatch_fanout8(world: &mut World, input: u64) {
    let reg = world.registry();

    fn s1(mut a: ResMut<ResA>, x: &u64) { a.0 = a.0.wrapping_add(*x); }
    fn s2(mut a: ResMut<ResB>, x: &u64) { a.0 = a.0.wrapping_add(*x as u32); }
    fn s3(mut a: ResMut<ResC>, x: &u64) { a.0 = a.0.wrapping_add(*x as u16); }
    fn s4(mut a: ResMut<ResD>, x: &u64) { a.0 = a.0.wrapping_add(*x as u8); }
    fn s5(mut a: ResMut<ResF>, x: &u64) { a.0 = a.0.wrapping_add(*x); }
    fn s6(mut a: ResMut<ResG>, x: &u64) { a.0 = a.0.wrapping_add(*x as u32); }
    fn s7(mut a: ResMut<ResH>, x: &u64) { a.0 = a.0.wrapping_add(*x as u16); }
    fn s8(mut a: ResMut<ResE>, x: &u64) { a.0 += *x as f64; }

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dispatch(fan_out!(
            s1.into_handler(&reg),
            s2.into_handler(&reg),
            s3.into_handler(&reg),
            s4.into_handler(&reg),
            s5.into_handler(&reg),
            s6.into_handler(&reg),
            s7.into_handler(&reg),
            s8.into_handler(&reg)
        ));
    p.run(world, input);
}

#[inline(never)]
pub fn pipe_dispatch_broadcast(world: &mut World, input: u64) {
    let reg = world.registry();

    fn sink_a(mut a: ResMut<ResA>, x: &u64) { a.0 = a.0.wrapping_add(*x); }
    fn sink_b(mut a: ResMut<ResB>, x: &u64) { a.0 = a.0.wrapping_add(*x as u32); }
    fn sink_c(mut a: ResMut<ResC>, x: &u64) { a.0 = a.0.wrapping_add(*x as u16); }

    let mut bc = Broadcast::<u64>::new();
    bc.add(sink_a.into_handler(&reg));
    bc.add(sink_b.into_handler(&reg));
    bc.add(sink_c.into_handler(&reg));

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dispatch(bc);
    p.run(world, input);
}

#[inline(never)]
pub fn pipe_dispatch_mid_then_continue(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let handler = consume_val.into_handler(&reg);
    // dispatch returns () — then we continue the chain from ()
    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dispatch(handler)
        .switch(|_w, _unit| 42u64);
    p.run(world, input)
}

// ---- Remaining section 12 gaps: 12.4, 12.6, 12.7, 12.11 ----

#[inline(never)]
pub fn pipe_world_local(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn count_calls(mut count: Local<u64>, x: u64) -> u64 {
        *count += 1;
        x.wrapping_add(*count)
    }

    let mut p = PipelineStart::<u64>::new()
        .then(count_calls, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_5_params(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(five_params, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_8_params(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(eight_params, &reg);
    p.run(world, input)
}

#[inline(never)]
pub fn pipe_world_changed_after(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn check_since(a: Res<ResA>, x: u64) -> u64 {
        // Compare against a fixed tick value.
        if a.changed_after(Sequence(0)) { x.wrapping_mul(2) } else { x }
    }

    let mut p = PipelineStart::<u64>::new()
        .then(write_res_a, &reg)
        .then(check_since, &reg);
    p.run(world, input)
}
