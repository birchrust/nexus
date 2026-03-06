//! Batch pipeline and DAG codegen audit cases.
//!
//! Categories 21-22 from the assembly audit plan.
//!
//! Batch variants own an input buffer. `run()` drains the buffer and processes
//! each item through the chain. The codegen question: does the compiler
//! generate the same quality inner loop as single-item dispatch?

#![allow(clippy::type_complexity)]
#![allow(unused_variables)]

use crate::dag::{DagArmStart, DagStart};
use crate::pipeline::PipelineStart;
use crate::{IntoHandler, World};
use super::helpers::*;

// ═══════════════════════════════════════════════════════════════════
// 21. Batch pipeline
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn batch_pipe_linear_3(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(add_three, &reg)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_linear_10(world: &mut World) {
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
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_guard(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .guard(|_w, x| *x > 10)
        .unwrap_or(0)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_option_chain(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(maybe_positive, &reg)
        .map(double, &reg)
        .filter(|_w, x| *x < 1000)
        .unwrap_or(0)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_result_chain(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(try_parse, &reg)
        .map(double, &reg)
        .unwrap_or(0)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_mixed_arity(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(add_res_a, &reg)
        .then(write_res_a, &reg)
        .then(add_both, &reg)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_splat(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(split_u64, &reg)
        .splat()
        .then(splat_add, &reg)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_route(world: &mut World) {
    let reg = world.registry();

    let on_true = PipelineStart::<u64>::new().then(double, &reg);
    let on_false = PipelineStart::<u64>::new().then(add_one, &reg);

    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .route(|_w, x| *x > 32, on_true, on_false)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_large(world: &mut World) {
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
        .then(consume_val, &reg)
        .build_batch(256);
    bp.input_mut().extend(0..256);
    bp.run(world);
}

// -- Batch early termination: guard/result → long dead chain per item --

#[inline(never)]
pub fn batch_pipe_guard_skip_10(world: &mut World) {
    let reg = world.registry();
    // Guard at step 1 → 10 maps. In the batch inner loop, items where
    // guard returns None should skip all 10 maps. Does the compiler
    // generate a tight branch per item?
    let mut bp = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 32)
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
        .unwrap_or(0)
        .then(consume_val, &reg)
        .build_batch(64);
    // Half the items will be filtered (0..32 fail guard, 33..64 pass).
    bp.input_mut().extend(0..64);
    bp.run(world);
}

#[inline(never)]
pub fn batch_pipe_res_skip_10(world: &mut World) {
    let reg = world.registry();
    // try_parse returns Err for input >= 10_000. In the batch loop,
    // Err items should skip all 10 maps.
    let mut bp = PipelineStart::<u64>::new()
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
        .unwrap_or(0)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ═══════════════════════════════════════════════════════════════════
// 22. Batch DAG
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn batch_dag_linear_3(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_add_three, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

#[inline(never)]
pub fn batch_dag_linear_10(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_add_three, &reg)
        .then(ref_square, &reg)
        .then(ref_sub_ten, &reg)
        .then(ref_shr_one, &reg)
        .then(ref_xor_mask, &reg)
        .then(ref_add_seven, &reg)
        .then(ref_triple, &reg)
        .then(ref_add_forty_two, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

#[inline(never)]
pub fn batch_dag_fork_merge(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

#[inline(never)]
pub fn batch_dag_guard(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .guard(|_w, x| *x > 10)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

#[inline(never)]
pub fn batch_dag_mixed_arity(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .then(ref_add_res_a, &reg)
        .then(ref_write_res_a, &reg)
        .then(ref_add_both, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

#[inline(never)]
pub fn batch_dag_diamond(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .merge(merge_add, &reg)
        .fork()
        .arm(|a| a.then(ref_square, &reg))
        .arm(|a| a.then(ref_shr_one, &reg))
        .merge(merge_mul, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

#[inline(never)]
pub fn batch_dag_large(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_add_three, &reg)
        .then(ref_square, &reg)
        .then(ref_sub_ten, &reg)
        .then(ref_shr_one, &reg)
        .then(ref_xor_mask, &reg)
        .then(ref_add_seven, &reg)
        .then(ref_triple, &reg)
        .then(ref_add_forty_two, &reg)
        .then(ref_consume, &reg)
        .build_batch(256);
    bd.input_mut().extend(0..256);
    bd.run(world);
}

// ═══════════════════════════════════════════════════════════════════
// Remaining batch pipeline gaps
// ═══════════════════════════════════════════════════════════════════

// ---- 21.3: guard + filter in batch ----

#[inline(never)]
pub fn batch_pipe_guard_filter(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .guard(|_w, x| *x > 10)
        .filter(|_w, x| *x < 1000)
        .unwrap_or(0)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ---- 21.6: type transition in batch ----

#[inline(never)]
pub fn batch_pipe_transition(world: &mut World) {
    let reg = world.registry();

    fn log_error(_err: u32) {}

    let mut bp = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .guard(|_w, x| *x > 0)
        .ok_or(0u32)
        .catch(log_error, &reg)
        .unwrap_or(0)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ---- 21.8: 3-way switch in batch ----

#[inline(never)]
pub fn batch_pipe_switch(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .switch(|_w, x| match x % 3 {
            0 => x.wrapping_mul(2),
            1 => x.wrapping_add(10),
            _ => x.wrapping_sub(5),
        })
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ---- 21.11: dispatch in batch ----

#[inline(never)]
pub fn batch_pipe_dispatch(world: &mut World) {
    let reg = world.registry();
    let handler = consume_val.into_handler(&reg);
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .dispatch(handler)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ---- 21.12: buffer reuse (two runs) ----

#[inline(never)]
pub fn batch_pipe_buffer_reuse(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(consume_val, &reg)
        .build_batch(64);
    // First run
    bp.input_mut().extend(0..64);
    bp.run(world);
    // Second run — buffer already allocated, no new allocation
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ---- 21.13: empty input ----

#[inline(never)]
pub fn batch_pipe_empty(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(add_three, &reg)
        .then(consume_val, &reg)
        .build_batch(64);
    // No extend — zero items
    bp.run(world);
}

// ---- 21.14: single item ----

#[inline(never)]
pub fn batch_pipe_single_item(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .then(add_one, &reg)
        .then(double, &reg)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().push(42);
    bp.run(world);
}

// ---- 21.15: drain codegen (trivial passthrough) ----

#[inline(never)]
pub fn batch_pipe_drain_codegen(world: &mut World) {
    let reg = world.registry();
    let mut bp = PipelineStart::<u64>::new()
        .switch(|_w, x| x)
        .then(consume_val, &reg)
        .build_batch(64);
    bp.input_mut().extend(0..64);
    bp.run(world);
}

// ═══════════════════════════════════════════════════════════════════
// Remaining batch DAG gaps
// ═══════════════════════════════════════════════════════════════════

// ---- 22.3: fork-4 in batch DAG ----

#[inline(never)]
pub fn batch_dag_fork4(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .arm(|a| a.then(ref_add_seven, &reg))
        .arm(|a| a.then(ref_xor_mask, &reg))
        .merge(merge_4, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

// ---- 22.4: nested fork in batch DAG ----

#[inline(never)]
pub fn batch_dag_nested_fork(world: &mut World) {
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

// ---- 22.5: option chain in batch DAG ----

#[inline(never)]
pub fn batch_dag_option_chain(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(maybe_positive, &reg)
        .map(ref_double, &reg)
        .filter(|_w, x| *x < 1000)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

// ---- 22.6: result chain in batch DAG ----

#[inline(never)]
pub fn batch_dag_result_chain(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(try_parse, &reg)
        .map(ref_double, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

// ---- 22.7: route in batch DAG ----

#[inline(never)]
pub fn batch_dag_route(world: &mut World) {
    let reg = world.registry();

    let on_true = DagArmStart::<u64>::new().then(ref_double, &reg);
    let on_false = DagArmStart::<u64>::new().then(ref_add_one, &reg);

    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .route(|_w, x| *x > 32, on_true, on_false)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

// ---- 22.8: splat in batch DAG ----

#[inline(never)]
pub fn batch_dag_splat(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(split_u64, &reg)
        .splat()
        .then(ref_splat_add, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

// ---- 22.9: heavy batch DAG (fork-4 with 5-step arms) ----

#[inline(never)]
pub fn batch_dag_heavy(world: &mut World) {
    let reg = world.registry();
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_double, &reg)
                .then(ref_add_three, &reg)
                .then(ref_square, &reg)
                .then(ref_sub_ten, &reg)
                .then(ref_shr_one, &reg)
        })
        .arm(|a| {
            a.then(ref_add_one, &reg)
                .then(ref_triple, &reg)
                .then(ref_xor_mask, &reg)
                .then(ref_add_seven, &reg)
                .then(ref_add_forty_two, &reg)
        })
        .arm(|a| {
            a.then(ref_triple, &reg)
                .then(ref_add_three, &reg)
                .then(ref_double, &reg)
                .then(ref_square, &reg)
                .then(ref_add_one, &reg)
        })
        .arm(|a| {
            a.then(ref_xor_mask, &reg)
                .then(ref_shr_one, &reg)
                .then(ref_add_seven, &reg)
                .then(ref_triple, &reg)
                .then(ref_sub_ten, &reg)
        })
        .merge(merge_4, &reg)
        .then(ref_consume, &reg)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}

// ---- 22.10: dispatch in batch DAG ----

#[inline(never)]
pub fn batch_dag_dispatch(world: &mut World) {
    let reg = world.registry();
    let handler = consume_val.into_handler(&reg);
    let mut bd = DagStart::<u64>::new()
        .root(add_one, &reg)
        .then(ref_double, &reg)
        .dispatch(handler)
        .build_batch(64);
    bd.input_mut().extend(0..64);
    bd.run(world);
}
