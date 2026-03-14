//! DAG codegen audit cases.
//!
//! Categories 13-20 from the assembly audit plan.
//!
//! Key difference from pipeline: `root()` takes by-value step, all subsequent
//! `.then()` calls take by-reference steps (`ref_*` helpers). DAGs produce
//! `Handler<E>` where `run()` returns `()`, so chains end with a sink step.

#![allow(clippy::type_complexity)]
#![allow(unused_variables)]

use crate::dag::{DagArmSeed, DagBuilder};
use crate::{Handler, IntoHandler, ResMut, World, fan_out, resolve_arm};
use super::helpers::*;

// ═══════════════════════════════════════════════════════════════════
// 13. DAG linear chains
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn dag_linear_1(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_linear_3(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_add_three, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_linear_5(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_add_three, &reg)
        .then(ref_square, &reg)
        .then(ref_sub_ten, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_linear_10(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
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
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_linear_20(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
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
        .then(ref_add_one, &reg)
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
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_linear_mixed_arity(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(ref_add_res_a, &reg)
        .then(ref_write_res_a, &reg)
        .then(ref_add_both, &reg)
        .then(ref_three_params, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 14. DAG fork & merge
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn dag_fork2_merge(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork3_merge(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .arm(|a| a.then(ref_add_seven, &reg))
        .merge(merge_3, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork4_merge(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .arm(|a| a.then(ref_add_seven, &reg))
        .arm(|a| a.then(ref_xor_mask, &reg))
        .merge(merge_4, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork2_join(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_consume, &reg))
        .arm(|a| a.then(ref_consume, &reg))
        .join()
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork3_join(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_consume, &reg))
        .arm(|a| a.then(ref_consume, &reg))
        .arm(|a| a.then(ref_consume, &reg))
        .join()
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork2_merge_consume(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .merge(merge_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork3_merge_consume(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .arm(|a| a.then(ref_add_seven, &reg))
        .merge(merge_3_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork_heavy_arms(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
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
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_nested_fork(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
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
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_diamond(world: &mut World, input: u64) {
    let reg = world.registry();
    // Diamond: root → fork(A, B) → merge → fork(C, D) → merge → sink
    let mut d = DagBuilder::<u64>::new()
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
        .build();
    d.run(world, input);
}

// -- Early termination stress: guard/result → long dead chain --

#[inline(never)]
pub fn dag_guard_skip_10(world: &mut World, input: u64) {
    let reg = world.registry();
    // Guard at step 1 → 10 maps on the Some path. If guard returns
    // None, all 10 maps should be skipped — single branch?
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .guard(|x: &u64| *x > 100, &reg)
        .map(ref_double, &reg)
        .map(ref_add_three, &reg)
        .map(ref_square, &reg)
        .map(ref_sub_ten, &reg)
        .map(ref_shr_one, &reg)
        .map(ref_xor_mask, &reg)
        .map(ref_add_seven, &reg)
        .map(ref_triple, &reg)
        .map(ref_add_forty_two, &reg)
        .map(ref_add_one, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_res_err_skip_10(world: &mut World, input: u64) {
    let reg = world.registry();
    // try_parse returns Err for large inputs. 10 result maps should
    // be dead code on the Err path.
    let mut d = DagBuilder::<u64>::new()
        .root(try_parse, &reg)
        .map(ref_add_one, &reg)
        .map(ref_double, &reg)
        .map(ref_add_three, &reg)
        .map(ref_square, &reg)
        .map(ref_sub_ten, &reg)
        .map(ref_shr_one, &reg)
        .map(ref_xor_mask, &reg)
        .map(ref_add_seven, &reg)
        .map(ref_triple, &reg)
        .map(ref_add_forty_two, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 15. DAG combinators (guard, filter, dedup, option, result, bool)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn dag_guard(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .guard(|x: &u64| *x > 10, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_filter(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .guard(|x: &u64| *x > 0, &reg)
        .filter(|x: &u64| *x < 1000, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_dedup(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .dedup()
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_opt_map(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(maybe_positive, &reg)
        .map(ref_double, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_opt_and_then(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(maybe_positive, &reg)
        .and_then(ref_maybe_positive, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_opt_chain(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(maybe_positive, &reg)
        .map(ref_double, &reg)
        .filter(|x: &u64| *x < 1000, &reg)
        .inspect(|_x: &u64| {}, &reg)
        .on_none(|| {}, &reg)
        .unwrap_or_else(|| 0, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_res_map(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(try_parse, &reg)
        .map(ref_double, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_res_and_then(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(try_parse, &reg)
        .and_then(ref_try_parse, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_res_chain(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(try_parse, &reg)
        .map(ref_double, &reg)
        .and_then(ref_try_parse, &reg)
        .inspect(|_v: &u64| {}, &reg)
        .inspect_err(|_e: &u32| {}, &reg)
        .map_err(|e: u32| e as u64, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_res_catch(world: &mut World, input: u64) {
    let reg = world.registry();

    fn log_error(_err: &u32) {}

    let mut d = DagBuilder::<u64>::new()
        .root(try_parse, &reg)
        .catch(log_error, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_res_or_else(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(try_parse, &reg)
        .or_else(|e: u32| Ok::<u64, u32>(e as u64), &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_bool_chain(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(is_even, &reg)
        .and(|| true, &reg)
        .or(|| false, &reg)
        .not()
        .xor(|| true, &reg)
        .then(|b: &bool| -> u64 { if *b { 1 } else { 0 } }, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 16. DAG route & switch
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn dag_route_basic(world: &mut World, input: u64) {
    let reg = world.registry();

    let on_true = DagArmSeed::<u64>::new().then(ref_double, &reg);
    let on_false = DagArmSeed::<u64>::new().then(ref_add_one, &reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .route(|x: &u64| *x > 100, &reg, on_true, on_false)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_route_heavy_arms(world: &mut World, input: u64) {
    let reg = world.registry();

    let arm_t = DagArmSeed::<u64>::new()
        .then(ref_double, &reg)
        .then(ref_add_three, &reg)
        .then(ref_square, &reg)
        .then(ref_sub_ten, &reg)
        .then(ref_shr_one, &reg);

    let arm_f = DagArmSeed::<u64>::new()
        .then(ref_add_one, &reg)
        .then(ref_triple, &reg)
        .then(ref_xor_mask, &reg)
        .then(ref_add_seven, &reg)
        .then(ref_add_forty_two, &reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .route(|x: &u64| *x > 100, &reg, arm_t, arm_f)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_switch_basic(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(|x: &u64| if *x > 100 { x.wrapping_mul(2) } else { x.wrapping_add(1) }, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_switch_3way(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(|x: &u64| match x % 3 {
            0 => x.wrapping_mul(2),
            1 => x.wrapping_add(10),
            _ => x.wrapping_sub(5),
        }, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_switch_resolve_arm(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut arm_big = resolve_arm(ref_double, &reg);
    let mut arm_small = resolve_arm(ref_add_one, &reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(move |world: &mut World, x: &u64| {
            if *x > 100 {
                arm_big(world, x)
            } else {
                arm_small(world, x)
            }
        }, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 17. DAG splat
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn dag_splat2(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(split_u64, &reg)
        .splat()
        .then(ref_splat_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_splat3(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(split_3, &reg)
        .splat()
        .then(ref_splat_3, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_splat_in_arm(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_split_u64, &reg)
                .splat()
                .then(ref_splat_add, &reg)
        })
        .arm(|a| a.then(ref_double, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 18. DAG tap, tee, dispatch
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn dag_tap(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .tap(|_x: &u64| {}, &reg)
        .then(ref_double, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_tee(world: &mut World, input: u64) {
    let reg = world.registry();

    let side = DagArmSeed::<u64>::new()
        .then(ref_consume, &reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .tee(side)
        .then(ref_double, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_tee_heavy(world: &mut World, input: u64) {
    let reg = world.registry();

    let side = DagArmSeed::<u64>::new()
        .then(ref_add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_add_three, &reg)
        .then(ref_square, &reg)
        .then(ref_consume, &reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .tee(side)
        .then(ref_double, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let handler = consume_val.into_handler(&reg);
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .dispatch(handler)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 19. DAG world interaction
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn dag_world_res(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_res_a, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_world_res_mut(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(write_res_a, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_world_mixed_chain(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(ref_add_res_a, &reg)
        .then(ref_write_res_a, &reg)
        .then(ref_double, &reg)
        .then(ref_add_both, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_world_3_params(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(three_params, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 20. DAG complex topologies
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn dag_wide_fan_out(world: &mut World, input: u64) {
    let reg = world.registry();
    // 4-way fan-out, all sinks
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_consume, &reg))
        .arm(|a| a.then(ref_consume, &reg))
        .arm(|a| a.then(ref_consume, &reg))
        .arm(|a| a.then(ref_consume, &reg))
        .join()
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_sequential_forks(world: &mut World, input: u64) {
    let reg = world.registry();
    // Three sequential fork-merges
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .merge(merge_add, &reg)
        .fork()
        .arm(|a| a.then(ref_square, &reg))
        .arm(|a| a.then(ref_shr_one, &reg))
        .merge(merge_mul, &reg)
        .fork()
        .arm(|a| a.then(ref_add_one, &reg))
        .arm(|a| a.then(ref_xor_mask, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork_with_guard(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .guard(|x: &u64| *x > 10, &reg)
        .unwrap_or(0)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_fork_with_route(world: &mut World, input: u64) {
    let reg = world.registry();

    let on_true = DagArmSeed::<u64>::new().then(ref_double, &reg);
    let on_false = DagArmSeed::<u64>::new().then(ref_triple, &reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .route(|x: &u64| *x > 100, &reg, on_true, on_false)
        .fork()
        .arm(|a| a.then(ref_square, &reg))
        .arm(|a| a.then(ref_shr_one, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_full_kitchen_sink(world: &mut World, input: u64) {
    let reg = world.registry();

    fn log_error(_err: &u32) {}

    let tee_side = DagArmSeed::<u64>::new()
        .then(ref_consume, &reg);

    let route_true = DagArmSeed::<u64>::new()
        .then(ref_double, &reg);
    let route_false = DagArmSeed::<u64>::new()
        .then(ref_triple, &reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .tap(|_x: &u64| {}, &reg)
        .then(ref_add_res_a, &reg)
        .guard(|x: &u64| *x > 0, &reg)
        .map(ref_double, &reg)
        .unwrap_or(0)
        .tee(tee_side)
        .then(ref_try_parse, &reg)
        .map(ref_add_one, &reg)
        .catch(log_error, &reg)
        .unwrap_or(0)
        .route(|x: &u64| *x > 100, &reg, route_true, route_false)
        .fork()
        .arm(|a| a.then(ref_square, &reg))
        .arm(|a| a.then(ref_shr_one, &reg))
        .merge(merge_add, &reg)
        .dedup()
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// Remaining gaps
// ═══════════════════════════════════════════════════════════════════

// ---- 13.2 ----

#[inline(never)]
pub fn dag_linear_2(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(ref_double, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 14.7: deep nested fork (3 levels) ----

#[inline(never)]
pub fn dag_deep_nested_fork(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_double, &reg)
                .fork()
                .arm(|b| {
                    b.then(ref_add_one, &reg)
                        .fork()
                        .arm(|c| c.then(ref_triple, &reg))
                        .arm(|c| c.then(ref_add_seven, &reg))
                        .merge(merge_add, &reg)
                })
                .arm(|b| b.then(ref_square, &reg))
                .merge(merge_add, &reg)
        })
        .arm(|a| a.then(ref_xor_mask, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 14.8: fork → merge → then → then (post-merge continuation) ----

#[inline(never)]
pub fn dag_fork_then_chain(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| a.then(ref_triple, &reg))
        .merge(merge_add, &reg)
        .then(ref_add_three, &reg)
        .then(ref_square, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 14.10: asymmetric arms (1, 3, 5 steps) ----

#[inline(never)]
pub fn dag_asymmetric_arms(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| a.then(ref_double, &reg))
        .arm(|a| {
            a.then(ref_triple, &reg)
                .then(ref_add_three, &reg)
                .then(ref_square, &reg)
        })
        .arm(|a| {
            a.then(ref_add_one, &reg)
                .then(ref_shr_one, &reg)
                .then(ref_xor_mask, &reg)
                .then(ref_add_seven, &reg)
                .then(ref_add_forty_two, &reg)
        })
        .merge(merge_3, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 14.11: fork arms with combinators ----

#[inline(never)]
pub fn dag_fork_arm_combinators(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_add_one, &reg)
                .guard(|x: &u64| *x > 10, &reg)
                .map(ref_double, &reg)
                .unwrap_or(0)
        })
        .arm(|a| {
            a.then(ref_triple, &reg)
                .guard(|x: &u64| *x > 5, &reg)
                .filter(|x: &u64| *x < 500, &reg)
                .unwrap_or_else(|| 0, &reg)
        })
        .arm(|a| a.then(ref_add_seven, &reg))
        .merge(merge_3, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 15.2: filter inside a fork arm ----

#[inline(never)]
pub fn dag_filter_in_arm(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_add_one, &reg)
                .guard(|x: &u64| *x > 10, &reg)
                .filter(|x: &u64| *x < 1000, &reg)
                .unwrap_or(0)
        })
        .arm(|a| a.then(ref_double, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 16.7: transition guard → ok_or → catch → unwrap ----

#[inline(never)]
pub fn dag_transition_guard_ok_or(world: &mut World, input: u64) {
    let reg = world.registry();

    fn log_error(_err: &u32) {}

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .guard(|x: &u64| *x > 0, &reg)
        .ok_or(0u32)
        .catch(log_error, &reg)
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 17.2: route inside a fork arm ----

#[inline(never)]
pub fn dag_route_in_arm(world: &mut World, input: u64) {
    let reg = world.registry();

    let on_true = DagArmSeed::<u64>::new().then(ref_double, &reg);
    let on_false = DagArmSeed::<u64>::new().then(ref_triple, &reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .fork()
        .arm(|a| {
            a.then(ref_add_one, &reg)
                .route(|x: &u64| *x > 50, &reg, on_true, on_false)
        })
        .arm(|a| a.then(ref_add_seven, &reg))
        .merge(merge_add, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 17.3: nested route on DAG chain (3-way via resolve_arm) ----

#[inline(never)]
pub fn dag_route_nested(world: &mut World, input: u64) {
    let reg = world.registry();

    let mut big = resolve_arm(ref_double, &reg);
    let mut mid = resolve_arm(ref_triple, &reg);
    let mut small = resolve_arm(ref_add_one, &reg);

    // 3-way branch via nested switch (same as pipe_route_nested_2 strategy)
    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(move |world: &mut World, x: &u64| {
            if *x > 100 {
                big(world, x)
            } else if *x > 50 {
                mid(world, x)
            } else {
                small(world, x)
            }
        }, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 18.5: 5-element splat on DAG chain ----

#[inline(never)]
pub fn dag_splat5(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut d = DagBuilder::<u64>::new()
        .root(split_5, &reg)
        .splat()
        .then(ref_splat_5, &reg)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

// ---- 19.4: dispatch with fan_out ----

#[inline(never)]
pub fn dag_dispatch_fanout(world: &mut World, input: u64) {
    let reg = world.registry();

    fn sink_a(mut a: ResMut<ResA>, x: &u64) { a.0 = a.0.wrapping_add(*x); }
    fn sink_b(mut a: ResMut<ResB>, x: &u64) { a.0 = a.0.wrapping_add(*x as u32); }

    let h1 = sink_a.into_handler(&reg);
    let h2 = sink_b.into_handler(&reg);

    let mut d = DagBuilder::<u64>::new()
        .root(add_one, &reg)
        .then(ref_double, &reg)
        .dispatch(fan_out!(h1, h2))
        .build();
    d.run(world, input);
}

// ---- 20.1-20.3: DAG clone transitions ----
// DAG .cloned() requires the chain output to be a reference type.
// Root steps must return &T for cloned() to apply.

fn leak_u64(x: u64) -> &'static u64 { Box::leak(Box::new(x.wrapping_add(1))) }
fn maybe_leak(x: u64) -> Option<&'static u64> {
    if x > 0 { Some(Box::leak(Box::new(x))) } else { None }
}
fn try_leak(x: u64) -> Result<&'static u64, u32> {
    if x < 10_000 { Ok(Box::leak(Box::new(x))) } else { Err(x as u32) }
}

#[inline(never)]
pub fn dag_cloned_bare(world: &mut World, input: u64) {
    let reg = world.registry();
    // Root returns &'static u64, .cloned() converts to u64
    let mut d = DagBuilder::<u64>::new()
        .root(leak_u64, &reg)
        .cloned()
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_cloned_option(world: &mut World, input: u64) {
    let reg = world.registry();
    // Root returns Option<&'static u64>, .cloned() → Option<u64>
    let mut d = DagBuilder::<u64>::new()
        .root(maybe_leak, &reg)
        .cloned()
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}

#[inline(never)]
pub fn dag_cloned_result(world: &mut World, input: u64) {
    let reg = world.registry();
    // Root returns Result<&'static u64, u32>, .cloned() → Result<u64, u32>
    let mut d = DagBuilder::<u64>::new()
        .root(try_leak, &reg)
        .cloned()
        .unwrap_or(0)
        .then(ref_consume, &reg)
        .build();
    d.run(world, input);
}
