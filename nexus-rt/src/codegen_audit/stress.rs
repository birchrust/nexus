//! Stress and pathological codegen audit cases.
//!
//! Category 25 from the assembly audit plan. These test the limits of
//! LLVM's inlining and optimization budgets under extreme pipeline/DAG
//! depth, width, and complexity.

#![allow(clippy::type_complexity)]
#![allow(unused_variables)]
// Stress tests intentionally pass large types by value to audit stack codegen.
#![allow(clippy::large_types_passed_by_value)]

use crate::dag::{DagArmStart, DagStart};
use crate::pipeline::PipelineStart;
use crate::{Handler, IntoHandler, World};
use super::helpers::*;

// ═══════════════════════════════════════════════════════════════════
// 25.1: 30-step linear pipeline
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_30_steps(world: &mut World, input: u64) -> u64 {
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
        // 11-20
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
        // 21-30
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

// ═══════════════════════════════════════════════════════════════════
// 25.2: 50-step linear pipeline
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_50_steps(world: &mut World, input: u64) -> u64 {
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
        // 11-20
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
        // 21-30
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
        // 31-40
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
        // 41-50
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

// ═══════════════════════════════════════════════════════════════════
// 25.3: all combinator types in one chain
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_all_combinators(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    let tee_side = DagArmStart::<u64>::new()
        .then(ref_consume, &reg);

    let on_true = PipelineStart::<u64>::new().then(double, &reg);
    let on_false = PipelineStart::<u64>::new().then(add_one, &reg);

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)                    // then
        .tap(|_w, _x| {})                       // tap
        .tee(tee_side)                           // tee
        .guard(|_w, x| *x > 0)                  // guard
        .filter(|_w, x| *x < 10000)             // filter
        .inspect(|_w, _x| {})                    // inspect (Option)
        .on_none(|_w| {})                        // on_none
        .ok_or(0u32)                             // ok_or
        .map(double, &reg)                       // map (Result)
        .inspect(|_w, _x| {})                    // inspect (Result)
        .inspect_err(|_w, _e| {})                // inspect_err
        .map_err(|_w, e| e)                      // map_err
        .catch(log_error, &reg)                  // catch
        .unwrap_or(0)                            // unwrap_or (Option)
        .then(add_three, &reg)                   // then again
        .then(is_even, &reg)                     // bool
        .and(|_w| true)                          // and
        .or(|_w| false)                          // or
        .not()                                   // not
        .xor(|_w| true)                          // xor
        .switch(|_w, b| if b { 100u64 } else { 0u64 }) // switch
        .route(|_w, x| *x > 50, on_true, on_false);    // route
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 25.6: repeated type transitions
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_transition_chain(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    // T → Option → Result → Option → T → Option → Result → Option → T
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        // Cycle 1: T → Option → Result → Option → T
        .guard(|_w, x| *x > 0)
        .ok_or(0u32)
        .catch(log_error, &reg)
        .unwrap_or(0)
        // Cycle 2: T → Option → Result → Option → T
        .guard(|_w, x| *x > 0)
        .ok_or(0u32)
        .catch(log_error, &reg)
        .unwrap_or(0)
        .then(add_one, &reg);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 25.7: 4-deep nested route (16 leaf paths)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_route_4_deep(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    // 16 leaf arms
    let l1 = PipelineStart::<u64>::new().then(add_one, &reg);
    let l2 = PipelineStart::<u64>::new().then(double, &reg);
    let l3 = PipelineStart::<u64>::new().then(add_three, &reg);
    let l4 = PipelineStart::<u64>::new().then(triple, &reg);
    let l5 = PipelineStart::<u64>::new().then(add_seven, &reg);
    let l6 = PipelineStart::<u64>::new().then(square, &reg);
    let l7 = PipelineStart::<u64>::new().then(sub_ten, &reg);
    let l8 = PipelineStart::<u64>::new().then(shr_one, &reg);

    // Level 3 (4 routes)
    let r3a = PipelineStart::<u64>::new().switch(|_w, x| x).route(|_w, x| *x > 10, l1, l2);
    let r3b = PipelineStart::<u64>::new().switch(|_w, x| x).route(|_w, x| *x > 20, l3, l4);
    let r3c = PipelineStart::<u64>::new().switch(|_w, x| x).route(|_w, x| *x > 30, l5, l6);
    let r3d = PipelineStart::<u64>::new().switch(|_w, x| x).route(|_w, x| *x > 40, l7, l8);

    // Level 2 (2 routes)
    let r2a = PipelineStart::<u64>::new().switch(|_w, x| x).route(|_w, x| *x > 50, r3a, r3b);
    let r2b = PipelineStart::<u64>::new().switch(|_w, x| x).route(|_w, x| *x > 60, r3c, r3d);

    // Level 1 (top)
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .route(|_w, x| *x > 100, r2a, r2b);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 25.9: DAG fork + route mix
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_dag_fork_route_mix(world: &mut World, input: u64) {
    let reg = world.registry();

    let on_true = DagArmStart::<u64>::new().then(ref_double, &reg);
    let on_false = DagArmStart::<u64>::new().then(ref_triple, &reg);

    let mut d = DagStart::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_add_one, &reg)
                .route(|_w, x| *x > 50, on_true, on_false)
        })
        .arm(|a| a.then(ref_add_seven, &reg))
        .arm(|a| a.then(ref_square, &reg))
        .merge(merge_3, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 25.10: 30-step batch pipeline
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_batch_pipe_30_steps(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
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
        .then(add_forty_two, &reg)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ═══════════════════════════════════════════════════════════════════
// 25.11: batch DAG with nested forks
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_batch_dag_nested(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_double, &reg)
                .fork()
                .arm(|b| b.then(ref_add_one, &reg))
                .arm(|b| b.then(ref_triple, &reg))
                .merge(merge_add, &reg)
        })
        .arm(|a| a.then(ref_add_seven, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

// ═══════════════════════════════════════════════════════════════════
// 25.13: large type (4096-byte value)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_large_type(world: &mut World, input: u64) -> [u8; 4096] {
    fn make_large(x: u64) -> [u8; 4096] {
        let mut arr = [0u8; 4096];
        arr[0] = x as u8;
        arr
    }

    fn touch_large(x: [u8; 4096]) -> [u8; 4096] {
        let mut out = x;
        out[1] = out[0].wrapping_add(1);
        out
    }

    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .then(make_large, &reg)
        .then(touch_large, &reg)
        .then(touch_large, &reg);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 25.14: many closures (10 closure-based combinators)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_many_closures(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .filter(|_w, x| *x < 10000)
        .filter(|_w, x| *x > 5)
        .inspect(|_w, _x| {})
        .on_none(|_w| {})
        .map(double, &reg)
        .filter(|_w, x| *x < 50000)
        .filter(|_w, x| *x > 1)
        .inspect(|_w, _x| {})
        .unwrap_or_else(|_w| 0)
        .then(add_one, &reg);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 25.15: DAG wide fork (4x4 = 16 leaves, 2 levels)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_dag_wide_fork(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagStart::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_double, &reg)
                .fork()
                .arm(|b| b.then(ref_add_one, &reg))
                .arm(|b| b.then(ref_triple, &reg))
                .arm(|b| b.then(ref_add_seven, &reg))
                .arm(|b| b.then(ref_xor_mask, &reg))
                .merge(merge_4, &reg)
        })
        .arm(|a| {
            a.then(ref_triple, &reg)
                .fork()
                .arm(|b| b.then(ref_square, &reg))
                .arm(|b| b.then(ref_shr_one, &reg))
                .arm(|b| b.then(ref_sub_ten, &reg))
                .arm(|b| b.then(ref_add_forty_two, &reg))
                .merge(merge_4, &reg)
        })
        .arm(|a| {
            a.then(ref_add_seven, &reg)
                .fork()
                .arm(|b| b.then(ref_double, &reg))
                .arm(|b| b.then(ref_add_three, &reg))
                .arm(|b| b.then(ref_triple, &reg))
                .arm(|b| b.then(ref_add_one, &reg))
                .merge(merge_4, &reg)
        })
        .arm(|a| {
            a.then(ref_xor_mask, &reg)
                .fork()
                .arm(|b| b.then(ref_shr_one, &reg))
                .arm(|b| b.then(ref_sub_ten, &reg))
                .arm(|b| b.then(ref_square, &reg))
                .arm(|b| b.then(ref_add_forty_two, &reg))
                .merge(merge_4, &reg)
        })
        .merge(merge_4, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 25.16: sequential splat operations
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_splat_chain(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();
    // split → splat → add → split → splat → add (two cycles)
    let mut p = PipelineStart::<u64>::new()
        .then(split_u64, &reg)
        .splat()
        .then(splat_add, &reg)
        .then(split_u64, &reg)
        .splat()
        .then(splat_add, &reg);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 25.17: dedup in batch
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_dedup_in_batch(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dedup()
        .map(double, &reg)
        .unwrap_or(0)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ═══════════════════════════════════════════════════════════════════
// 25.18: tee inside a route arm
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_pipe_tee_in_route(world: &mut World, input: u64) -> u64 {
    let reg = world.registry();

    let tee_side = DagArmStart::<u64>::new()
        .then(ref_add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_consume, &reg);

    let arm_t = PipelineStart::<u64>::new()
        .then(double, &reg)
        .tee(tee_side)
        .then(add_three, &reg);

    let arm_f = PipelineStart::<u64>::new()
        .then(add_one, &reg);

    let mut p = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .route(|_w, x| *x > 100, arm_t, arm_f);
    p.run(world, input)
}

// ═══════════════════════════════════════════════════════════════════
// 25.19: pipeline kitchen sink
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn stress_mixed_everything(world: &mut World, input: u64) {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    let tee_side = DagArmStart::<u64>::new()
        .then(ref_consume, &reg);

    let on_true = PipelineStart::<u64>::new().then(double, &reg);
    let on_false = PipelineStart::<u64>::new().then(add_one, &reg);

    let handler = consume_val.into_handler(&reg);

    let mut p = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .tap(|_w, _x| {})
        .tee(tee_side)
        .guard(|_w, x| *x > 0)
        .map(double, &reg)
        .filter(|_w, x| *x < 100_000)
        .ok_or(0u32)
        .map(add_three, &reg)
        .catch(log_error, &reg)
        .unwrap_or(0)
        .route(|_w, x| *x > 50, on_true, on_false)
        .then(split_u64, &reg)
        .splat()
        .then(splat_add, &reg)
        .dispatch(handler);
    p.run(world, input);
}
