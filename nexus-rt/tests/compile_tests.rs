//! Comprehensive compile-time and runtime integration tests for nexus-rt
//! pipeline and DAG APIs.
//!
//! If this file compiles, the public API surface works for real users.
//! Each test exercises a specific pattern and verifies runtime behavior.

//! These are compile-time + runtime integration tests — helper functions
//! intentionally use specific signatures to exercise the nexus-rt API
//! (pass-by-value Params, trivially-copyable refs, items-after-statements
//! for test locality, f64 assert_eq for exact bit patterns, etc.).
#![allow(
    clippy::unnecessary_wraps,
    clippy::needless_pass_by_value,
    clippy::trivially_copy_pass_by_ref,
    clippy::items_after_statements,
    clippy::float_cmp,
    clippy::many_single_char_names,
    clippy::option_if_let_else,
    clippy::redundant_closure,
    clippy::manual_assert,
)]

use nexus_rt::dag::{DagArmStart, DagStart};
use nexus_rt::shutdown::Shutdown;
use nexus_rt::{
    Handler, IntoHandler, Local, PipelineStart, Res, ResMut, Seq, SeqMut, Virtual, World,
    WorldBuilder, resolve_arm, resolve_producer, resolve_ref_step, resolve_step,
};

// =========================================================================
// Helper types and named functions used across tests
// =========================================================================

#[derive(Debug, Clone, PartialEq)]
struct Order {
    id: u64,
    price: f64,
    size: u32,
}

impl Order {
    fn new(id: u64, price: f64, size: u32) -> Self {
        Self { id, price, size }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ValidOrder {
    id: u64,
    price: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct EnrichedOrder {
    id: u64,
    total: f64,
}

#[derive(Debug, Clone)]
struct MyError(String);

// -- Pipeline step functions (named fns, as required for Param resolution) --

fn identity_u32(x: u32) -> u32 {
    x
}

fn double_u32(x: u32) -> u64 {
    x as u64 * 2
}

fn add_ten(x: u32) -> u32 {
    x + 10
}

fn triple(x: u32) -> u32 {
    x * 3
}

fn store_u64(mut out: ResMut<u64>, val: u64) {
    *out = val;
}

fn read_factor_and_multiply(factor: Res<u64>, x: u32) -> u64 {
    *factor * x as u64
}

fn write_and_transform(mut out: ResMut<u64>, x: u32) -> u32 {
    *out = x as u64;
    x * 2
}

fn read_and_write(config: Res<u64>, mut out: ResMut<String>, x: u32) {
    *out = format!("{}:{}", *config, x);
}

fn opt_res_step(opt: Option<Res<u64>>, x: u32) -> u32 {
    match opt {
        Some(v) => x + *v as u32,
        None => x,
    }
}

fn opt_res_mut_step(opt: Option<ResMut<String>>, x: u32) -> u32 {
    if let Some(mut s) = opt {
        *s = x.to_string();
    }
    x
}

fn seq_step(seq: Seq, x: u32) -> u32 {
    let _ = seq.get();
    x
}

fn seq_mut_step(mut seq: SeqMut, x: u32) -> u32 {
    let _ = seq.advance();
    x
}

fn shutdown_step(shutdown: Shutdown, x: u32) -> u32 {
    let _ = shutdown.is_shutdown();
    x
}

fn validate_order(order: Order) -> Option<ValidOrder> {
    if order.price > 0.0 {
        Some(ValidOrder {
            id: order.id,
            price: order.price,
        })
    } else {
        None
    }
}

fn enrich_order(vo: ValidOrder) -> EnrichedOrder {
    EnrichedOrder {
        id: vo.id,
        total: vo.price * 2.0,
    }
}

fn store_enriched(mut out: ResMut<f64>, eo: EnrichedOrder) {
    *out = eo.total;
}

fn guard_positive(x: &u32) -> bool {
    *x > 0
}

fn guard_positive_with_res(threshold: Res<u32>, x: &u32) -> bool {
    *x > *threshold
}

fn tap_log(_x: &u32) {}

fn tap_log_with_res(_counter: Res<u64>, _x: &u32) {}

fn filter_even(x: &u32) -> bool {
    *x % 2 == 0
}

fn inspect_option(x: &u32) {
    let _ = *x;
}

fn produce_true() -> bool {
    true
}

fn produce_false() -> bool {
    false
}

fn fallible_parse(x: u32) -> Result<u64, MyError> {
    if x < 100 {
        Ok(x as u64)
    } else {
        Err(MyError("too large".into()))
    }
}

fn map_ok_double(x: u64) -> u64 {
    x * 2
}

fn and_then_validate(x: u64) -> Result<u64, MyError> {
    if x < 200 {
        Ok(x)
    } else {
        Err(MyError("too large after double".into()))
    }
}

fn catch_error(_err: MyError) {}

fn map_err_to_string(err: MyError) -> String {
    err.0
}

fn inspect_err_log(_err: &MyError) {}

fn inspect_ok_log(_val: &u64) {}

fn or_else_recover(_err: MyError) -> Result<u64, String> {
    Ok(0)
}

fn unwrap_or_else_result(err: MyError) -> u64 {
    let _ = err;
    42
}

fn splat2(a: u32, b: u32) -> u32 {
    a + b
}

fn splat3(a: u32, b: u32, c: u32) -> u32 {
    a + b + c
}

fn splat4(a: u32, b: u32, c: u32, d: u32) -> u32 {
    a + b + c + d
}

fn splat5(a: u32, b: u32, c: u32, d: u32, e: u32) -> u32 {
    a + b + c + d + e
}

fn make_pair(x: u32) -> (u32, u32) {
    (x, x + 1)
}

fn make_triple(x: u32) -> (u32, u32, u32) {
    (x, x + 1, x + 2)
}

fn make_quad(x: u32) -> (u32, u32, u32, u32) {
    (x, x + 1, x + 2, x + 3)
}

fn make_quint(x: u32) -> (u32, u32, u32, u32, u32) {
    (x, x + 1, x + 2, x + 3, x + 4)
}

fn store_u32(mut out: ResMut<u32>, val: u32) {
    *out = val;
}

// -- DAG step functions (takes &T) --

fn dag_double(x: &u32) -> u64 {
    *x as u64 * 2
}

fn dag_negate(x: &u32) -> i64 {
    -(*x as i64)
}

fn dag_store_u64(mut out: ResMut<u64>, val: &u64) {
    *out = *val;
}

fn dag_store_i64(mut out: ResMut<i64>, val: &i64) {
    *out = *val;
}

fn dag_add_one(x: &u64) -> u64 {
    *x + 1
}

fn dag_merge_sum(a: &u64, b: &i64) -> f64 {
    *a as f64 + *b as f64
}

fn dag_merge3(a: &u64, b: &i64, c: &f64) -> f64 {
    *a as f64 + *b as f64 + *c
}

fn dag_merge4(a: &u64, b: &u64, c: &u64, d: &u64) -> u64 {
    *a + *b + *c + *d
}

fn dag_store_f64(mut out: ResMut<f64>, val: &f64) {
    *out = *val;
}

fn dag_guard_positive(x: &u64) -> bool {
    *x > 0
}

fn dag_tap_noop(_x: &u64) {}

fn dag_id(x: u32) -> u32 {
    x
}

fn dag_store_u32(mut out: ResMut<u32>, val: &u32) {
    *out = *val;
}

fn dag_splat2(a: &u32, b: &u32) -> u32 {
    *a + *b
}

// Helper to build a simple world with common resources
fn build_world() -> World {
    let mut wb = WorldBuilder::new();
    wb.register::<u32>(0);
    wb.register::<u64>(0);
    wb.register::<i64>(0);
    wb.register::<f64>(0.0);
    wb.register::<String>(String::new());
    wb.register::<bool>(false);
    wb.build()
}

// =========================================================================
// 1. Pipeline basics
// =========================================================================

#[test]
fn pipeline_single_step() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(store_u32, r)
        .build();
    p.run(&mut world, 42);
    assert_eq!(*world.resource::<u32>(), 42);
}

#[test]
fn pipeline_linear_chain_three() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(add_ten, r)
        .then(triple, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 1);
    assert_eq!(*world.resource::<u32>(), 33); // (1+10)*3
}

#[test]
fn pipeline_linear_chain_five() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .then(add_ten, r)
        .then(triple, r)
        .then(add_ten, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 0);
    // 0 -> 0 -> 10 -> 30 -> 40
    assert_eq!(*world.resource::<u32>(), 40);
}

#[test]
fn pipeline_build_batch() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut batch = PipelineStart::<u32>::new()
        .then(
            |x: u32| -> u64 { x as u64 },
            r,
        )
        .then(store_u64, r)
        .build_batch(16);

    batch.input_mut().extend_from_slice(&[1, 2, 3]);
    batch.run(&mut world);
    // last item wins
    assert_eq!(*world.resource::<u64>(), 3);
    assert!(batch.input().is_empty());
}

#[test]
fn pipeline_run_direct() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut builder = PipelineStart::<u32>::new()
        .then(double_u32, r);

    let result = builder.run(&mut world, 5);
    assert_eq!(result, 10);
}

// =========================================================================
// 2. Pipeline with every Param type
// =========================================================================

#[test]
fn pipeline_with_res() {
    let mut wb = WorldBuilder::new();
    wb.register::<u64>(10);
    let mut world = wb.build();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(read_factor_and_multiply, r)
        .then(store_u64, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 50);
}

#[test]
fn pipeline_with_res_mut() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(write_and_transform, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 7);
    assert_eq!(*world.resource::<u64>(), 7);
    assert_eq!(*world.resource::<u32>(), 14);
}

#[test]
fn pipeline_with_multiple_res_and_res_mut() {
    let mut wb = WorldBuilder::new();
    wb.register::<u64>(42);
    wb.register::<String>(String::new());
    let mut world = wb.build();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(read_and_write, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(world.resource::<String>().as_str(), "42:5");
}

#[test]
fn pipeline_with_option_res() {
    let mut wb = WorldBuilder::new();
    wb.register::<u64>(100);
    let mut world = wb.build();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(opt_res_step, r);
    let result = p.run(&mut world, 5);
    assert_eq!(result, 105);
}

#[test]
fn pipeline_with_option_res_mut() {
    let mut wb = WorldBuilder::new();
    wb.register::<String>(String::new());
    let mut world = wb.build();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(opt_res_mut_step, r);
    let result = p.run(&mut world, 7);
    assert_eq!(result, 7);
    assert_eq!(world.resource::<String>().as_str(), "7");
}

#[test]
fn pipeline_with_seq() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(seq_step, r);
    let result = p.run(&mut world, 5);
    assert_eq!(result, 5);
}

#[test]
fn pipeline_with_seq_mut() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(seq_mut_step, r);
    let result = p.run(&mut world, 5);
    assert_eq!(result, 5);
}

#[test]
fn pipeline_with_shutdown() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(shutdown_step, r);
    let result = p.run(&mut world, 5);
    assert_eq!(result, 5);
}

// =========================================================================
// 3. Pipeline Option combinators
// =========================================================================

#[test]
fn pipeline_option_map() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(|x: u32| -> Option<u32> { Some(x) }, r)
        .map(double_u32, r)
        .map(|x: u64| { let _ = x; }, r)
        .build();
    p.run(&mut world, 5);
}

#[test]
fn pipeline_guard_then_map() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);

    // zero gets guarded out
    p.run(&mut world, 0);
    assert_eq!(*world.resource::<u32>(), 5); // unchanged
}

#[test]
fn pipeline_filter() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(|_: &u32| true, r) // enter Option land
        .filter(filter_even, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 4);
    assert_eq!(*world.resource::<u32>(), 4);

    p.run(&mut world, 5); // odd, filtered
    assert_eq!(*world.resource::<u32>(), 4); // unchanged
}

#[test]
fn pipeline_inspect_option() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .inspect(inspect_option, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 3);
    assert_eq!(*world.resource::<u32>(), 3);
}

#[test]
fn pipeline_and_then_option() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(|x: u32| -> Option<u32> { Some(x) }, r)
        .and_then(|x: u32| -> Option<u64> { Some(x as u64 * 3) }, r)
        .map(store_u64, r)
        .build();
    p.run(&mut world, 4);
    assert_eq!(*world.resource::<u64>(), 12);
}

#[test]
fn pipeline_on_none() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .on_none(|| {}, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 0); // guarded, on_none fires
    assert_eq!(*world.resource::<u32>(), 0); // unchanged, was default
}

#[test]
fn pipeline_ok_or() {
    let mut world = build_world();
    let r = world.registry_mut();
    // ok_or produces Result<u32, &str>; catch takes the error by value
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .ok_or("was zero")
        .map(store_u32, r)
        .catch(|_err: &str| {}, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn pipeline_unwrap_or_option() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .unwrap_or(99)
        .then(store_u32, r)
        .build();

    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);

    p.run(&mut world, 0);
    assert_eq!(*world.resource::<u32>(), 99);
}

#[test]
fn pipeline_cloned_option() {
    let mut world = build_world();
    let r = world.registry_mut();
    // Option<&T> -> Option<T> via .cloned() — T must be Sized + Clone
    static YES: u64 = 1;
    static NO: u64 = 0;
    let mut p = PipelineStart::<u32>::new()
        .then(|x: u32| -> Option<u32> { Some(x) }, r)
        .map(|x: u32| -> &'static u64 { if x > 0 { &YES } else { &NO } }, r)
        .cloned()
        .map(|val: u64| { let _ = val; }, r)
        .build();
    p.run(&mut world, 1);
}

// =========================================================================
// 4. Pipeline Result combinators
// =========================================================================

#[test]
fn pipeline_result_map() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .map(map_ok_double, r)
        .map(store_u64, r)
        .catch(catch_error, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 10);
}

#[test]
fn pipeline_result_and_then() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .and_then(and_then_validate, r)
        .map(store_u64, r)
        .catch(catch_error, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

#[test]
fn pipeline_result_catch() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .catch(catch_error, r)
        .map(store_u64, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);

    // Error case: catch consumes the error, None produced
    p.run(&mut world, 200);
    assert_eq!(*world.resource::<u64>(), 5); // unchanged
}

#[test]
fn pipeline_result_map_err() {
    let mut world = build_world();
    let r = world.registry_mut();
    // After map_err, error type is String. Pipeline catch takes E by value.
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .map_err(map_err_to_string, r)
        .catch(|_err: String| {}, r)
        .map(store_u64, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

#[test]
fn pipeline_result_inspect_err() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .inspect_err(inspect_err_log, r)
        .map(store_u64, r)
        .catch(catch_error, r)
        .build();
    p.run(&mut world, 200); // error path
}

#[test]
fn pipeline_result_ok() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .ok()
        .map(store_u64, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

#[test]
fn pipeline_result_unwrap_or() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .unwrap_or(999)
        .then(store_u64, r)
        .build();
    p.run(&mut world, 200);
    assert_eq!(*world.resource::<u64>(), 999);
}

#[test]
fn pipeline_result_or_else() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .or_else(or_else_recover, r)
        .map(store_u64, r)
        .catch(|_err: String| {}, r)
        .build();
    p.run(&mut world, 200); // error -> recovered to Ok(0)
    assert_eq!(*world.resource::<u64>(), 0);
}

#[test]
fn pipeline_result_unwrap_or_else() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .unwrap_or_else(unwrap_or_else_result, r)
        .then(store_u64, r)
        .build();
    p.run(&mut world, 200);
    assert_eq!(*world.resource::<u64>(), 42);
}

#[test]
fn pipeline_result_inspect_ok() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .inspect(inspect_ok_log, r)
        .map(store_u64, r)
        .catch(catch_error, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

// =========================================================================
// 5. Pipeline bool combinators
// =========================================================================

#[test]
fn pipeline_bool_not() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(|x: u32| -> bool { x > 5 }, r)
        .not();
    assert!(p.run(&mut world, 3)); // 3 > 5 is false, !false = true
    assert!(!p.run(&mut world, 10)); // 10 > 5 is true, !true = false
}

#[test]
fn pipeline_bool_and() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(|x: u32| -> bool { x > 5 }, r)
        .and(produce_true, r);
    assert!(p.run(&mut world, 10)); // true && true
    assert!(!p.run(&mut world, 3)); // false && true (short-circuits)
}

#[test]
fn pipeline_bool_or() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(|x: u32| -> bool { x > 5 }, r)
        .or(produce_true, r);
    assert!(p.run(&mut world, 3)); // false || true
    assert!(p.run(&mut world, 10)); // true || true (short-circuits)
}

#[test]
fn pipeline_bool_xor() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(|x: u32| -> bool { x > 5 }, r)
        .xor(produce_false, r);
    assert!(!p.run(&mut world, 3)); // false ^ false = false
    assert!(p.run(&mut world, 10)); // true ^ false = true
}

// =========================================================================
// 6. Pipeline special combinators
// =========================================================================

#[test]
fn pipeline_guard_with_res_param() {
    let mut wb = WorldBuilder::new();
    wb.register::<u32>(5);
    wb.register::<u64>(0);
    let mut world = wb.build();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive_with_res, r)
        .map(|x: u32| x as u64, r)
        .map(store_u64, r)
        .build();

    p.run(&mut world, 10); // 10 > 5, passes guard
    assert_eq!(*world.resource::<u64>(), 10);

    p.run(&mut world, 3); // 3 > 5 is false, guarded out
    assert_eq!(*world.resource::<u64>(), 10); // unchanged
}

#[test]
fn pipeline_guard_arity0_closure() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(|x: &u32| *x > 10, r)
        .map(store_u32, r)
        .build();

    p.run(&mut world, 20);
    assert_eq!(*world.resource::<u32>(), 20);

    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 20); // unchanged
}

#[test]
fn pipeline_tap_named_fn() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .tap(tap_log, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 7);
    assert_eq!(*world.resource::<u32>(), 7);
}

#[test]
fn pipeline_tap_arity0_closure() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .tap(|_x: &u32| {}, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 9);
    assert_eq!(*world.resource::<u32>(), 9);
}

#[test]
fn pipeline_route() {
    let mut world = build_world();
    let r = world.registry_mut();

    let large = PipelineStart::new().then(|x: u32| x * 10, r);
    let small = PipelineStart::new().then(|x: u32| x, r);

    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .route(|x: &u32| *x > 100, r, large, small)
        .then(store_u32, r)
        .build();

    p.run(&mut world, 200);
    assert_eq!(*world.resource::<u32>(), 2000);

    p.run(&mut world, 50);
    assert_eq!(*world.resource::<u32>(), 50);
}

#[test]
fn pipeline_tee() {
    let mut world = build_world();
    let r = world.registry_mut();

    let side = DagArmStart::<u32>::new()
        .then(|x: &u32| *x as u64, r)
        .then(dag_store_u64, r);

    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .tee(side)
        .then(store_u32, r)
        .build();

    p.run(&mut world, 7);
    assert_eq!(*world.resource::<u32>(), 7);
    assert_eq!(*world.resource::<u64>(), 7);
}

#[test]
fn pipeline_dedup() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .dedup()
        .map(store_u32, r)
        .build();

    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);

    p.run(&mut world, 5); // duplicate, suppressed
    // store not called again, stays 5

    p.run(&mut world, 10);
    assert_eq!(*world.resource::<u32>(), 10);
}

#[test]
fn pipeline_scan() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .scan(0u64, |acc: &mut u64, val: u32| {
            *acc += val as u64;
            *acc
        }, r)
        .then(store_u64, r)
        .build();

    p.run(&mut world, 1);
    assert_eq!(*world.resource::<u64>(), 1);

    p.run(&mut world, 2);
    assert_eq!(*world.resource::<u64>(), 3);

    p.run(&mut world, 3);
    assert_eq!(*world.resource::<u64>(), 6);
}

#[test]
fn pipeline_dispatch_to_handler() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn sink(mut out: ResMut<u64>, val: u64) {
        *out = val;
    }
    let handler = sink.into_handler(r);

    let mut p = PipelineStart::<u32>::new()
        .then(double_u32, r)
        .dispatch(handler)
        .build();

    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 10);
}

// =========================================================================
// 7. Pipeline splat
// =========================================================================

#[test]
fn pipeline_splat2() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(make_pair, r)
        .splat()
        .then(splat2, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 11); // 5 + 6
}

#[test]
fn pipeline_splat3() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(make_triple, r)
        .splat()
        .then(splat3, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 18); // 5+6+7
}

#[test]
fn pipeline_splat4() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(make_quad, r)
        .splat()
        .then(splat4, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 1);
    assert_eq!(*world.resource::<u32>(), 10); // 1+2+3+4
}

#[test]
fn pipeline_splat5() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(make_quint, r)
        .splat()
        .then(splat5, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 0);
    assert_eq!(*world.resource::<u32>(), 10); // 0+1+2+3+4
}

#[test]
fn pipeline_splat_at_start() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<(u32, u32)>::new()
        .splat()
        .then(splat2, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, (3, 4));
    assert_eq!(*world.resource::<u32>(), 7);
}

// =========================================================================
// 8. Pipeline Opaque closures
// =========================================================================

#[test]
fn pipeline_guard_opaque() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(|_w: &mut World, x: &u32| -> bool { *x > 5 }, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 10);
    assert_eq!(*world.resource::<u32>(), 10);
    p.run(&mut world, 3);
    assert_eq!(*world.resource::<u32>(), 10); // unchanged
}

#[test]
fn pipeline_tap_opaque() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .tap(|_w: &mut World, _x: &u32| {}, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn pipeline_on_none_opaque() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .on_none(|_w: &mut World| {}, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 0); // guarded out, on_none fires
}

#[test]
fn pipeline_then_opaque() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(|w: &mut World, x: u32| {
            *w.resource_mut::<u64>() = x as u64;
        }, r)
        .build();
    p.run(&mut world, 42);
    assert_eq!(*world.resource::<u64>(), 42);
}

// =========================================================================
// 9. Pipeline Output<()> terminal
// =========================================================================

#[test]
fn pipeline_option_unit_terminal() {
    let mut world = build_world();
    let r = world.registry_mut();
    // Chain ends with Option<()> -- build() should work
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn pipeline_filter_then_map_sink() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(|_: &u32| true, r)
        .filter(filter_even, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 4);
    assert_eq!(*world.resource::<u32>(), 4);
}

// =========================================================================
// 10. Pipeline borrowed events
// =========================================================================

#[test]
fn pipeline_borrowed_slice() {
    let mut world = build_world();
    let data = vec![1u8, 2, 3, 4, 5];
    let r = world.registry_mut();

    fn decode(data: &[u8]) -> u32 {
        data.len() as u32
    }

    let mut p = PipelineStart::<&[u8]>::new()
        .then(decode, r)
        .then(store_u32, r)
        .build();

    p.run(&mut world, &data);
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn pipeline_borrowed_str() {
    let mut world = build_world();
    let msg = String::from("hello");
    let r = world.registry_mut();

    fn parse_len(s: &str) -> u32 {
        s.len() as u32
    }

    let mut p = PipelineStart::<&str>::new()
        .then(parse_len, r)
        .then(store_u32, r)
        .build();

    p.run(&mut world, msg.as_str());
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn pipeline_borrowed_option_unit_terminal() {
    let mut world = build_world();
    let data = vec![1u8, 2, 3];
    let r = world.registry_mut();

    fn decode_len(data: &[u8]) -> u32 {
        data.len() as u32
    }

    let mut p = PipelineStart::<&[u8]>::new()
        .then(decode_len, r)
        .guard(guard_positive, r)
        .map(store_u32, r)
        .build();

    p.run(&mut world, &data);
    assert_eq!(*world.resource::<u32>(), 3);
}

#[test]
fn pipeline_borrowed_through_guard() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn parse_val(s: &str) -> u32 {
        s.len() as u32
    }

    let mut p = PipelineStart::<&str>::new()
        .then(parse_val, r)
        .guard(guard_positive, r)
        .filter(filter_even, r)
        .map(store_u32, r)
        .build();

    // &str literal has 'static lifetime, no drop ordering issue
    p.run(&mut world, "abcd"); // len=4, positive, even
    assert_eq!(*world.resource::<u32>(), 4);
}

#[test]
fn pipeline_borrowed_run_direct() {
    let mut world = build_world();
    let data = vec![1u8, 2, 3, 4];
    let r = world.registry_mut();

    fn decode_len(data: &[u8]) -> u32 {
        data.len() as u32
    }

    let mut builder = PipelineStart::<&[u8]>::new()
        .then(decode_len, r);

    let result = builder.run(&mut world, &data);
    assert_eq!(result, 4);
}

#[test]
fn pipeline_to_boxed_handler() {
    let mut world = build_world();
    let r = world.registry_mut();

    let p = PipelineStart::<u32>::new()
        .then(store_u32, r)
        .build();

    let mut boxed: Virtual<u32> = Box::new(p);
    boxed.run(&mut world, 77);
    assert_eq!(*world.resource::<u32>(), 77);
}

// =========================================================================
// 11. DAG basics
// =========================================================================

#[test]
fn dag_root_then_build() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut d = DagStart::<u32>::new()
        .root(dag_id, r)
        .then(dag_store_u32, r)
        .build();
    d.run(&mut world, 42);
    assert_eq!(*world.resource::<u32>(), 42);
}

#[test]
fn dag_root_then_then_build() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root_to_u64(x: u32) -> u64 {
        x as u64
    }

    let mut d = DagStart::<u32>::new()
        .root(root_to_u64, r)
        .then(dag_add_one, r)
        .then(dag_store_u64, r)
        .build();
    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 6);
}

#[test]
fn dag_root_single_step() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut d = DagStart::<u32>::new()
        .root(|x: u32| { let _ = x; }, r)
        .build();
    d.run(&mut world, 1);
}

#[test]
fn dag_fork_merge_2arm() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| a.then(dag_double, r))
        .arm(|a| a.then(dag_negate, r))
        .merge(dag_merge_sum, r)
        .then(dag_store_f64, r)
        .build();

    d.run(&mut world, 10);
    // arm0: 10*2=20, arm1: -10, merge: 20+(-10)=10.0
    assert_eq!(*world.resource::<f64>(), 10.0);
}

#[test]
fn dag_fork_join_2arm() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| a.then(|x: &u32| *x as u64, r).then(dag_store_u64, r))
        .arm(|a| a.then(|x: &u32| -(*x as i64), r).then(dag_store_i64, r))
        .join()
        .build();

    d.run(&mut world, 7);
    assert_eq!(*world.resource::<u64>(), 7);
    assert_eq!(*world.resource::<i64>(), -7);
}

#[test]
fn dag_build_batch() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u64 { x as u64 }
    fn accumulate(mut sum: ResMut<u64>, val: &u64) {
        *sum += *val;
    }

    let mut batch = DagStart::<u32>::new()
        .root(root, r)
        .then(accumulate, r)
        .build_batch(8);

    batch.input_mut().extend([1, 2, 3]);
    batch.run(&mut world);
    assert_eq!(*world.resource::<u64>(), 6); // 1+2+3
    assert!(batch.input().is_empty());
}

// =========================================================================
// 12. DAG fork patterns
// =========================================================================

#[test]
fn dag_fork_merge_3arm() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| a.then(dag_double, r))
        .arm(|a| a.then(dag_negate, r))
        .arm(|a| a.then(|x: &u32| *x as f64 * 0.5, r))
        .merge(dag_merge3, r)
        .then(dag_store_f64, r)
        .build();

    d.run(&mut world, 10);
    // arm0: 20, arm1: -10, arm2: 5.0
    // merge: 20 + (-10) + 5.0 = 15.0
    assert_eq!(*world.resource::<f64>(), 15.0);
}

#[test]
fn dag_fork_merge_4arm() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u64 { x as u64 }
    fn arm_fn(x: &u64) -> u64 { *x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| a.then(arm_fn, r))
        .arm(|a| a.then(arm_fn, r))
        .arm(|a| a.then(arm_fn, r))
        .arm(|a| a.then(arm_fn, r))
        .merge(dag_merge4, r)
        .then(dag_store_u64, r)
        .build();

    d.run(&mut world, 3);
    assert_eq!(*world.resource::<u64>(), 12); // 3*4
}

#[test]
fn dag_fork_arms_with_multiple_steps() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| a.then(dag_double, r).then(dag_add_one, r))
        .arm(|a| a.then(dag_negate, r))
        .merge(dag_merge_sum, r)
        .then(dag_store_f64, r)
        .build();

    d.run(&mut world, 5);
    // arm0: double=10, add_one=11
    // arm1: negate=-5
    // merge: 11 + (-5) = 6.0
    assert_eq!(*world.resource::<f64>(), 6.0);
}

#[test]
fn dag_fork_arm_with_guard() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u64 { x as u64 }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| {
            a.then(|x: &u64| *x, r)
                .guard(dag_guard_positive, r)
                .unwrap_or(0)
        })
        .arm(|a| a.then(|x: &u64| *x + 100, r))
        .merge(|a: &u64, b: &u64| *a + *b, r)
        .then(dag_store_u64, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 110); // 5 + 105
}

#[test]
fn dag_nested_fork() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| {
            a.then(|x: &u32| *x as u64, r)
                .fork()
                .arm(|inner| inner.then(|x: &u64| *x * 2, r))
                .arm(|inner| inner.then(|x: &u64| *x * 3, r))
                .merge(|a: &u64, b: &u64| (*a + *b) as f64, r)
        })
        .arm(|a| a.then(|x: &u32| *x as f64 * 10.0, r))
        .merge(|a: &f64, b: &f64| *a + *b, r)
        .then(dag_store_f64, r)
        .build();

    d.run(&mut world, 2);
    // inner arm0: 2*2=4, inner arm1: 2*3=6, inner merge: 10.0
    // outer arm1: 2*10=20.0
    // outer merge: 10.0 + 20.0 = 30.0
    assert_eq!(*world.resource::<f64>(), 30.0);
}

// =========================================================================
// 13. DAG splat
// =========================================================================

#[test]
fn dag_splat_chain() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn split(x: u32) -> (u32, u32) {
        (x, x + 1)
    }

    let mut d = DagStart::<u32>::new()
        .root(split, r)
        .splat()
        .then(dag_splat2, r)
        .then(dag_store_u32, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 11); // 5 + 6
}

#[test]
fn dag_splat_inside_arm() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| {
            a.then(|x: &u32| (*x, *x + 1), r)
                .splat()
                .then(dag_splat2, r)
        })
        .arm(|a| a.then(|x: &u32| *x * 10, r))
        .merge(|a: &u32, b: &u32| (*a + *b) as u64, r)
        .then(dag_store_u64, r)
        .build();

    d.run(&mut world, 3);
    // arm0: splat(3,4) -> 7
    // arm1: 30
    // merge: 37
    assert_eq!(*world.resource::<u64>(), 37);
}

// =========================================================================
// 14. DAG borrowed events
// =========================================================================

#[test]
fn dag_borrowed_slice() {
    let mut world = build_world();
    let data = vec![1u8, 2, 3];
    let r = world.registry_mut();

    fn decode(data: &[u8]) -> u32 {
        data.len() as u32
    }

    let mut d = DagStart::<&[u8]>::new()
        .root(decode, r)
        .then(dag_store_u32, r)
        .build();

    d.run(&mut world, &data);
    assert_eq!(*world.resource::<u32>(), 3);
}

#[test]
fn dag_borrowed_through_fork() {
    let mut world = build_world();
    let data = vec![1u8, 2, 3, 4, 5];
    let r = world.registry_mut();

    fn decode(data: &[u8]) -> u32 {
        data.len() as u32
    }

    let mut d = DagStart::<&[u8]>::new()
        .root(decode, r)
        .fork()
        .arm(|a| a.then(dag_double, r).then(dag_store_u64, r))
        .arm(|a| a.then(dag_negate, r).then(dag_store_i64, r))
        .join()
        .build();

    d.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 10); // 5 * 2
    assert_eq!(*world.resource::<i64>(), -5);
}

#[test]
fn dag_borrowed_with_guard() {
    let mut world = build_world();
    let data = vec![1u8, 2, 3];
    let short = vec![1u8];
    let r = world.registry_mut();

    fn decode(data: &[u8]) -> u32 {
        data.len() as u32
    }

    let mut d = DagStart::<&[u8]>::new()
        .root(decode, r)
        .guard(|x: &u32| *x > 2, r)
        .unwrap_or(0)
        .then(dag_store_u32, r)
        .build();

    d.run(&mut world, &data);
    assert_eq!(*world.resource::<u32>(), 3);

    d.run(&mut world, &short);
    assert_eq!(*world.resource::<u32>(), 0); // guarded, unwrap_or
}

// =========================================================================
// 15. DAG Option<()> terminal
// =========================================================================

#[test]
fn dag_option_unit_terminal() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .map(dag_store_u32, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);
}

// =========================================================================
// 16. DAG route
// =========================================================================

#[test]
fn dag_route() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let fast = DagArmStart::new().then(|x: &u32| *x as u64 * 100, r);
    let slow = DagArmStart::new().then(|x: &u32| *x as u64, r);

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .route(|x: &u32| *x > 10, r, fast, slow)
        .then(dag_store_u64, r)
        .build();

    d.run(&mut world, 20);
    assert_eq!(*world.resource::<u64>(), 2000);

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

// =========================================================================
// 17. Mixed patterns
// =========================================================================

#[test]
fn pipeline_dispatch_handler_interop() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn handler_fn(mut out: ResMut<u64>, event: u64) {
        *out = event * 10;
    }
    let handler = handler_fn.into_handler(r);

    let mut p = PipelineStart::<u32>::new()
        .then(double_u32, r)
        .dispatch(handler)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 100); // 5*2=10, handler: 10*10=100
}

#[test]
fn pipeline_result_catch_then() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(fallible_parse, r)
        .catch(catch_error, r)
        .map(store_u64, r)
        .build();

    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

#[test]
fn pipeline_guard_unwrap_then() {
    // Common validation pattern: guard -> unwrap_or -> then -> build
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .unwrap_or(0)
        .then(|x: u32| x as u64, r)
        .then(store_u64, r)
        .build();

    p.run(&mut world, 10);
    assert_eq!(*world.resource::<u64>(), 10);

    p.run(&mut world, 0);
    assert_eq!(*world.resource::<u64>(), 0);
}

#[test]
fn pipeline_realistic_decode_validate_enrich_store() {
    let mut wb = WorldBuilder::new();
    wb.register::<f64>(0.0);
    let mut world = wb.build();
    let r = world.registry_mut();

    let mut p = PipelineStart::<Order>::new()
        .then(validate_order, r)
        .and_then(|vo: ValidOrder| -> Option<EnrichedOrder> {
            Some(enrich_order(vo))
        }, r)
        .map(store_enriched, r)
        .build();

    p.run(&mut world, Order::new(1, 10.0, 100));
    assert_eq!(*world.resource::<f64>(), 20.0);

    // Invalid order (price=0) gets None from validate, skipped
    p.run(&mut world, Order::new(2, 0.0, 50));
    assert_eq!(*world.resource::<f64>(), 20.0); // unchanged
}

#[test]
fn pipeline_long_realistic() {
    // decode -> validate -> enrich -> route -> sink
    let mut wb = WorldBuilder::new();
    wb.register::<f64>(0.0);
    wb.register::<u64>(0);
    let mut world = wb.build();
    let r = world.registry_mut();

    fn decode(raw: u32) -> Order {
        Order::new(raw as u64, raw as f64, raw)
    }

    fn validate(order: Order) -> Result<Order, MyError> {
        if order.price > 0.0 {
            Ok(order)
        } else {
            Err(MyError("bad price".into()))
        }
    }

    fn log_error(_err: MyError) {}

    fn store_price(mut out: ResMut<f64>, order: Order) {
        *out = order.price;
    }

    let mut p = PipelineStart::<u32>::new()
        .then(decode, r)
        .then(validate, r)
        .catch(log_error, r)
        .map(store_price, r)
        .build();

    p.run(&mut world, 42);
    assert_eq!(*world.resource::<f64>(), 42.0);
}

// =========================================================================
// 18. Handler interop
// =========================================================================

#[test]
fn pipeline_build_into_virtual() {
    let mut world = build_world();
    let r = world.registry_mut();

    let pipeline = PipelineStart::<u32>::new()
        .then(store_u32, r)
        .build();

    let mut v: Virtual<u32> = Box::new(pipeline);
    v.run(&mut world, 99);
    assert_eq!(*world.resource::<u32>(), 99);
}

#[test]
fn dag_build_into_virtual() {
    let mut world = build_world();
    let r = world.registry_mut();

    let dag = DagStart::<u32>::new()
        .root(dag_id, r)
        .then(dag_store_u32, r)
        .build();

    let mut v: Virtual<u32> = Box::new(dag);
    v.run(&mut world, 88);
    assert_eq!(*world.resource::<u32>(), 88);
}

// =========================================================================
// 19. resolve_step / resolve_ref_step / resolve_producer helpers
// =========================================================================

#[test]
fn resolve_step_named_fn() {
    let mut world = build_world();
    let r = world.registry();

    let mut step = resolve_step(double_u32, r);
    let result = step(&mut world, 7);
    assert_eq!(result, 14);
}

#[test]
fn resolve_step_arity0_closure() {
    let mut world = build_world();
    let r = world.registry();

    let mut step = resolve_step(|x: u32| x + 100, r);
    let result = step(&mut world, 5);
    assert_eq!(result, 105);
}

#[test]
fn resolve_ref_step_named_fn() {
    let mut world = build_world();
    let r = world.registry();

    let mut step = resolve_ref_step(guard_positive, r);
    assert!(step(&mut world, &5));
    assert!(!step(&mut world, &0));
}

#[test]
fn resolve_producer_helper() {
    let mut world = build_world();
    let r = world.registry();

    let mut prod = resolve_producer(produce_true, r);
    assert!(prod(&mut world));
}

#[test]
fn resolve_arm_helper() {
    let mut world = build_world();
    let r = world.registry();

    fn dag_step(x: &u32) -> u64 {
        *x as u64 * 3
    }

    let mut arm = resolve_arm(dag_step, r);
    let result = arm(&mut world, &10);
    assert_eq!(result, 30);
}

// =========================================================================
// 20. Batch patterns
// =========================================================================

#[test]
fn batch_pipeline_fill_run_check() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn accumulate(mut sum: ResMut<u64>, x: u32) {
        *sum += x as u64;
    }

    let mut batch = PipelineStart::<u32>::new()
        .then(accumulate, r)
        .build_batch(32);

    assert!(batch.input().is_empty());
    batch.input_mut().extend_from_slice(&[10, 20, 30]);
    assert_eq!(batch.input().len(), 3);
    batch.run(&mut world);
    assert!(batch.input().is_empty());
    assert_eq!(*world.resource::<u64>(), 60);
}

#[test]
fn batch_dag_fill_run_check() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u64 { x as u64 }
    fn accumulate(mut sum: ResMut<u64>, val: &u64) {
        *sum += *val;
    }

    let mut batch = DagStart::<u32>::new()
        .root(root, r)
        .then(accumulate, r)
        .build_batch(32);

    assert!(batch.input().is_empty());
    batch.input_mut().extend([5, 10, 15]);
    assert_eq!(batch.input().len(), 3);
    batch.run(&mut world);
    assert!(batch.input().is_empty());
    assert_eq!(*world.resource::<u64>(), 30);
}

// =========================================================================
// Additional edge case tests
// =========================================================================

#[test]
fn pipeline_scan_at_start() {
    let mut world = build_world();
    let r = world.registry_mut();

    let mut p = PipelineStart::<u32>::new()
        .scan(0u64, |acc: &mut u64, x: u32| {
            *acc += x as u64;
            *acc
        }, r)
        .then(store_u64, r)
        .build();

    p.run(&mut world, 1);
    assert_eq!(*world.resource::<u64>(), 1);
    p.run(&mut world, 2);
    assert_eq!(*world.resource::<u64>(), 3);
}

#[test]
fn dag_scan() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .scan(0u64, |acc: &mut u64, x: &u32| {
            *acc += *x as u64;
            *acc
        }, r)
        .then(dag_store_u64, r)
        .build();

    d.run(&mut world, 10);
    assert_eq!(*world.resource::<u64>(), 10);
    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 15);
}

#[test]
fn dag_dedup() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .dedup()
        .map(dag_store_u32, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);

    d.run(&mut world, 5); // duplicate, suppressed
    d.run(&mut world, 10);
    assert_eq!(*world.resource::<u32>(), 10);
}

#[test]
fn dag_bool_not_and() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> bool { x > 5 }
    fn store_bool(mut out: ResMut<bool>, val: &bool) {
        *out = *val;
    }

    // Test: !root && produce_true — store the result
    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .not()
        .and(produce_true, r)
        .then(store_bool, r)
        .build();

    d.run(&mut world, 3); // 3>5=false, !false=true, true&&true
    assert!(*world.resource::<bool>());

    d.run(&mut world, 10); // 10>5=true, !true=false, false&&true (short-circuits)
    assert!(!*world.resource::<bool>());
}

#[test]
fn dag_tap() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u64 { x as u64 }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .tap(dag_tap_noop, r)
        .then(dag_store_u64, r)
        .build();

    d.run(&mut world, 7);
    assert_eq!(*world.resource::<u64>(), 7);
}

#[test]
fn dag_tee() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let side = DagArmStart::<u32>::new()
        .then(|x: &u32| *x as u64, r)
        .then(dag_store_u64, r);

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .tee(side)
        .then(dag_store_u32, r)
        .build();

    d.run(&mut world, 9);
    assert_eq!(*world.resource::<u32>(), 9);
    assert_eq!(*world.resource::<u64>(), 9);
}

#[test]
fn dag_result_combinators() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> Result<u64, MyError> {
        if x < 100 {
            Ok(x as u64)
        } else {
            Err(MyError("too large".into()))
        }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .map(|x: &u64| *x * 2, r)
        .catch(|_err: &MyError| {}, r)
        .map(dag_store_u64, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 10);

    d.run(&mut world, 200); // error path
    assert_eq!(*world.resource::<u64>(), 10); // unchanged
}

#[test]
fn dag_option_combinators() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .filter(|x: &u32| *x % 2 == 0, r)
        .inspect(|_x: &u32| {}, r)
        .map(dag_store_u32, r)
        .build();

    d.run(&mut world, 4);
    assert_eq!(*world.resource::<u32>(), 4);

    d.run(&mut world, 3); // odd, filtered
    assert_eq!(*world.resource::<u32>(), 4);

    d.run(&mut world, 0); // guarded
    assert_eq!(*world.resource::<u32>(), 4);
}

#[test]
fn dag_option_ok_or() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .ok_or("zero")
        .map(dag_store_u32, r)
        .catch(|_e: &&str| {}, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn dag_option_unwrap_or() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .unwrap_or(99)
        .then(dag_store_u32, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);

    d.run(&mut world, 0);
    assert_eq!(*world.resource::<u32>(), 99);
}

#[test]
fn dag_result_ok() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> Result<u64, MyError> {
        if x > 0 { Ok(x as u64) } else { Err(MyError("zero".into())) }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .ok()
        .map(dag_store_u64, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

#[test]
fn dag_result_unwrap_or() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> Result<u64, MyError> {
        if x > 0 { Ok(x as u64) } else { Err(MyError("zero".into())) }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .unwrap_or(999)
        .then(dag_store_u64, r)
        .build();

    d.run(&mut world, 0);
    assert_eq!(*world.resource::<u64>(), 999);
}

#[test]
fn dag_result_map_err() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> Result<u64, MyError> {
        if x > 0 { Ok(x as u64) } else { Err(MyError("zero".into())) }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .map_err(|e: MyError| e.0, r)
        .ok()
        .map(dag_store_u64, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

#[test]
fn dag_result_inspect_err() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> Result<u64, MyError> {
        if x > 0 { Ok(x as u64) } else { Err(MyError("zero".into())) }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .inspect_err(|_e: &MyError| {}, r)
        .ok()
        .map(dag_store_u64, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 5);
}

#[test]
fn dag_result_or_else() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> Result<u64, MyError> {
        if x > 0 { Ok(x as u64) } else { Err(MyError("zero".into())) }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .or_else(|_e: MyError| -> Result<u64, String> { Ok(0) }, r)
        .ok()
        .map(dag_store_u64, r)
        .build();

    d.run(&mut world, 0);
    assert_eq!(*world.resource::<u64>(), 0);
}

#[test]
fn dag_dispatch() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn handler_fn(mut out: ResMut<u64>, event: u64) {
        *out = event;
    }
    let handler = handler_fn.into_handler(r);

    fn root(x: u32) -> u64 { x as u64 * 3 }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .dispatch(handler)
        .build();

    d.run(&mut world, 7);
    assert_eq!(*world.resource::<u64>(), 21);
}

#[test]
fn pipeline_ok_or_else() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .ok_or_else(|| "was zero".to_string(), r)
        .catch(|_err: String| {}, r)
        .map(store_u32, r)
        .build();
    p.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn pipeline_unwrap_or_else_option() {
    let mut world = build_world();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .unwrap_or_else(|| 42, r)
        .then(store_u32, r)
        .build();

    p.run(&mut world, 0);
    assert_eq!(*world.resource::<u32>(), 42);

    p.run(&mut world, 7);
    assert_eq!(*world.resource::<u32>(), 7);
}

#[test]
fn dag_option_on_none() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .on_none(|| {}, r)
        .map(dag_store_u32, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn dag_option_ok_or_else() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .ok_or_else(|| "zero".to_string(), r)
        .ok()
        .map(dag_store_u32, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u32>(), 5);
}

#[test]
fn dag_option_unwrap_or_else() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .unwrap_or_else(|| 42, r)
        .then(dag_store_u32, r)
        .build();

    d.run(&mut world, 0);
    assert_eq!(*world.resource::<u32>(), 42);
}

#[test]
fn dag_result_unwrap_or_else() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> Result<u64, MyError> {
        if x > 0 { Ok(x as u64) } else { Err(MyError("zero".into())) }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .unwrap_or_else(|_e: MyError| 999, r)
        .then(dag_store_u64, r)
        .build();

    d.run(&mut world, 0);
    assert_eq!(*world.resource::<u64>(), 999);
}

#[test]
fn dag_result_and_then() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> Result<u64, MyError> {
        if x > 0 { Ok(x as u64) } else { Err(MyError("zero".into())) }
    }

    fn validate(x: &u64) -> Result<u64, MyError> {
        if *x < 100 { Ok(*x * 2) } else { Err(MyError("too large".into())) }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .and_then(validate, r)
        .ok()
        .map(dag_store_u64, r)
        .build();

    d.run(&mut world, 5);
    assert_eq!(*world.resource::<u64>(), 10);
}

#[test]
fn dag_bool_or() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> bool { x > 5 }
    fn store_bool(mut out: ResMut<bool>, val: &bool) {
        *out = *val;
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .or(produce_false, r)
        .then(store_bool, r)
        .build();

    d.run(&mut world, 10); // true || false
    assert!(*world.resource::<bool>());

    d.run(&mut world, 3); // false || false
    assert!(!*world.resource::<bool>());
}

#[test]
fn dag_bool_xor() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> bool { x > 5 }
    fn store_bool(mut out: ResMut<bool>, val: &bool) {
        *out = *val;
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .xor(produce_true, r)
        .then(store_bool, r)
        .build();

    d.run(&mut world, 10); // true ^ true = false
    assert!(!*world.resource::<bool>());

    d.run(&mut world, 3); // false ^ true = true
    assert!(*world.resource::<bool>());
}

#[test]
fn batch_pipeline_option_terminal() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn accumulate(mut sum: ResMut<u64>, x: u32) {
        *sum += x as u64;
    }

    let mut batch = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .guard(guard_positive, r)
        .map(accumulate, r)
        .build_batch(16);

    batch.input_mut().extend_from_slice(&[0, 1, 2, 3]);
    batch.run(&mut world);
    // 0 is guarded out, 1+2+3 = 6
    assert_eq!(*world.resource::<u64>(), 6);
}

#[test]
fn dag_batch_option_terminal() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }
    fn accumulate(mut sum: ResMut<u64>, x: &u32) {
        *sum += *x as u64;
    }

    let mut batch = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .map(accumulate, r)
        .build_batch(16);

    batch.input_mut().extend([0, 1, 2, 3]);
    batch.run(&mut world);
    assert_eq!(*world.resource::<u64>(), 6);
}

#[test]
fn dag_join_3arm() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| a.then(|x: &u32| { let _ = *x; }, r))
        .arm(|a| a.then(|x: &u32| { let _ = *x; }, r))
        .arm(|a| a.then(dag_store_u32, r))
        .join()
        .build();

    d.run(&mut world, 11);
    assert_eq!(*world.resource::<u32>(), 11);
}

#[test]
fn dag_join_4arm() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .fork()
        .arm(|a| a.then(|x: &u32| { let _ = *x; }, r))
        .arm(|a| a.then(|x: &u32| { let _ = *x; }, r))
        .arm(|a| a.then(|x: &u32| { let _ = *x; }, r))
        .arm(|a| a.then(dag_store_u32, r))
        .join()
        .build();

    d.run(&mut world, 22);
    assert_eq!(*world.resource::<u32>(), 22);
}

#[test]
fn pipeline_tap_with_res() {
    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    wb.register::<u32>(0);
    let mut world = wb.build();
    let r = world.registry_mut();
    let mut p = PipelineStart::<u32>::new()
        .then(identity_u32, r)
        .tap(tap_log_with_res, r)
        .then(store_u32, r)
        .build();
    p.run(&mut world, 3);
    assert_eq!(*world.resource::<u32>(), 3);
}

#[test]
fn dag_option_and_then() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn root(x: u32) -> u32 { x }

    fn validate(x: &u32) -> Option<u64> {
        if *x > 5 { Some(*x as u64) } else { None }
    }

    let mut d = DagStart::<u32>::new()
        .root(root, r)
        .guard(|x: &u32| *x > 0, r)
        .and_then(validate, r)
        .map(dag_store_u64, r)
        .build();

    d.run(&mut world, 10);
    assert_eq!(*world.resource::<u64>(), 10);
}

#[test]
fn pipeline_multiple_batch_runs() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn accumulate(mut sum: ResMut<u64>, x: u32) {
        *sum += x as u64;
    }

    let mut batch = PipelineStart::<u32>::new()
        .then(accumulate, r)
        .build_batch(16);

    batch.input_mut().extend_from_slice(&[1, 2, 3]);
    batch.run(&mut world);
    assert_eq!(*world.resource::<u64>(), 6);

    batch.input_mut().extend_from_slice(&[4, 5]);
    batch.run(&mut world);
    assert_eq!(*world.resource::<u64>(), 15);
}

// =========================================================================
// 21. HRTB boxing — borrowed event dispatch
// =========================================================================
//
// These tests prove that Pipeline and Dag can be boxed as
// `Box<dyn for<'a> Handler<&'a T>>` for zero-copy event dispatch with
// borrowed data. Primarily compile-time tests — if they compile, the HRTB
// bounds are satisfied. Runtime assertions verify dispatch correctness.
//
// NOT tested (documented reasons):
// - BatchPipeline/BatchDag with borrowed events: Batch stores items in
//   Vec<In>, requires In: 'static. Can't store &'a T in a Vec.
// - Templates with borrowed events: Blueprint::Event is an associated
//   type, can't express HRTB at the type level.

// -- HRTB helper types and step functions --

#[derive(Debug)]
#[allow(dead_code)]
struct Message<'a> {
    topic: u8,
    payload: &'a [u8],
}

fn slice_len(data: &[u8]) -> usize { data.len() }
fn store_len(mut out: ResMut<u64>, len: usize) { *out = len as u64; }

fn msg_payload_len(msg: Message<'_>) -> usize { msg.payload.len() }

fn hrtb_dag_double_len(len: &usize) -> usize { *len * 2 }
fn hrtb_dag_store_len(mut out: ResMut<u64>, len: &usize) { *out = *len as u64; }
fn hrtb_dag_add_lens(a: &usize, b: &usize) -> usize { *a + *b }

#[test]
fn hrtb_pipeline_basic() {
    let mut world = build_world();
    let r = world.registry_mut();

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .then(store_len, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    let data = vec![1u8, 2, 3, 4, 5];
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 5);
}

#[test]
fn hrtb_pipeline_with_guard() {
    let mut world = build_world();
    let r = world.registry_mut();

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .guard(|len: &usize| *len > 2, r)
        .map(store_len, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    let data = vec![1u8, 2, 3];
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 3);

    // Short slice — guard filters, store_len not called, value unchanged
    let short = vec![1u8];
    boxed.run(&mut world, &short);
    assert_eq!(*world.resource::<u64>(), 3);
}

#[test]
fn hrtb_pipeline_with_option_chain() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn mark_none(mut flag: ResMut<bool>) { *flag = true; }

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .guard(|len: &usize| *len > 0, r)
        .map(store_len, r)
        .on_none(mark_none, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    let data = vec![10u8, 20, 30];
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 3);
    assert!(!*world.resource::<bool>()); // on_none did NOT fire

    // Empty — guard rejects, on_none fires, store not called
    let empty: Vec<u8> = vec![];
    boxed.run(&mut world, &empty);
    assert_eq!(*world.resource::<u64>(), 3); // unchanged
    assert!(*world.resource::<bool>()); // on_none DID fire
}

#[test]
fn hrtb_pipeline_with_closure() {
    let mut world = build_world();
    let r = world.registry_mut();

    // Arity-0 closures in .then() and .guard() positions (not just guard)
    let pipeline = PipelineStart::<&[u8]>::new()
        .then(|data: &[u8]| data.len() * 2, r)
        .guard(|doubled: &usize| *doubled > 0, r)
        .map(|val: usize| { let _ = val; }, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    let data = vec![7u8, 8];
    boxed.run(&mut world, &data);
    // Compiles + runs — arity-0 closures compose through HRTB
}

#[test]
fn hrtb_dag_basic() {
    let mut world = build_world();
    let r = world.registry_mut();

    let dag = DagStart::<&[u8]>::new()
        .root(slice_len, r)
        .then(hrtb_dag_store_len, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(dag);

    let data = vec![1u8, 2, 3];
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 3);
}

#[test]
fn hrtb_dag_fork_merge() {
    let mut world = build_world();
    let r = world.registry_mut();

    let dag = DagStart::<&[u8]>::new()
        .root(slice_len, r)
        .fork()
        .arm(|a| a.then(hrtb_dag_double_len, r))
        .arm(|a| a.then(|len: &usize| *len + 10, r))
        .merge(hrtb_dag_add_lens, r)
        .then(hrtb_dag_store_len, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(dag);

    let data = vec![1u8, 2, 3, 4, 5]; // len=5
    boxed.run(&mut world, &data);
    // arm0: 5*2=10, arm1: 5+10=15, merge: 10+15=25
    assert_eq!(*world.resource::<u64>(), 25);
}

#[test]
fn hrtb_dag_fork_join() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn store_len_u32(mut out: ResMut<u32>, len: &usize) { *out = *len as u32; }
    fn store_len_i64(mut out: ResMut<i64>, len: &usize) { *out = *len as i64; }

    let dag = DagStart::<&[u8]>::new()
        .root(slice_len, r)
        .fork()
        .arm(|a| a.then(store_len_u32, r))
        .arm(|a| a.then(store_len_i64, r))
        .join()
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(dag);

    let data = vec![1u8, 2, 3, 4];
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u32>(), 4);
    assert_eq!(*world.resource::<i64>(), 4);
}

#[test]
fn hrtb_borrowed_struct_event() {
    let mut world = build_world();
    let r = world.registry_mut();

    let pipeline = PipelineStart::<Message<'_>>::new()
        .then(msg_payload_len, r)
        .then(store_len, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<Message<'a>>> = Box::new(pipeline);

    let payload = vec![10u8, 20, 30, 40];
    let msg = Message { topic: 1, payload: &payload };
    boxed.run(&mut world, msg);
    assert_eq!(*world.resource::<u64>(), 4);
}

#[test]
fn hrtb_dispatch_map() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn store_len_u32(mut out: ResMut<u32>, len: usize) { *out = len as u32; }

    let p1 = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .then(store_len, r)
        .build();

    let p2 = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .then(store_len_u32, r)
        .build();

    type HrtbSliceHandler = Box<dyn for<'a> Handler<&'a [u8]>>;
    let mut map: std::collections::HashMap<u8, HrtbSliceHandler> =
        std::collections::HashMap::new();
    map.insert(0, Box::new(p1));
    map.insert(1, Box::new(p2));

    let data = vec![1u8, 2, 3];
    map.get_mut(&0).unwrap().run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 3);

    let data2 = vec![10u8, 20];
    map.get_mut(&1).unwrap().run(&mut world, &data2);
    assert_eq!(*world.resource::<u32>(), 2);
}

#[test]
fn hrtb_direct_run_no_boxing() {
    let mut world = build_world();
    let r = world.registry_mut();

    let mut pipeline = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .then(store_len, r)
        .build();

    let data = vec![1u8, 2, 3, 4, 5, 6];
    pipeline.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 6);
}

#[test]
fn hrtb_dag_direct_run_no_boxing() {
    let mut world = build_world();
    let r = world.registry_mut();

    let mut dag = DagStart::<&[u8]>::new()
        .root(slice_len, r)
        .then(hrtb_dag_double_len, r)
        .then(hrtb_dag_store_len, r)
        .build();

    let data = vec![1u8, 2, 3];
    dag.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 6); // 3 * 2
}

#[test]
fn hrtb_pipeline_and_dag_in_same_map() {
    let mut world = build_world();
    let r = world.registry_mut();

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .then(store_len, r)
        .build();

    let dag = DagStart::<&[u8]>::new()
        .root(slice_len, r)
        .then(hrtb_dag_double_len, r)
        .then(hrtb_dag_store_len, r)
        .build();

    type HrtbSliceHandler = Box<dyn for<'a> Handler<&'a [u8]>>;
    let mut map: std::collections::HashMap<u8, HrtbSliceHandler> =
        std::collections::HashMap::new();
    map.insert(0, Box::new(pipeline));
    map.insert(1, Box::new(dag));

    let data = vec![1u8, 2, 3, 4];
    map.get_mut(&0).unwrap().run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 4);

    let data2 = vec![10u8, 20, 30];
    map.get_mut(&1).unwrap().run(&mut world, &data2);
    assert_eq!(*world.resource::<u64>(), 6); // 3 * 2
}

#[test]
fn hrtb_disjoint_lifetimes() {
    let mut world = build_world();
    let r = world.registry_mut();

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .then(store_len, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    // First dispatch — borrow from a scope that ends
    {
        let data = vec![1u8, 2, 3];
        boxed.run(&mut world, &data);
        assert_eq!(*world.resource::<u64>(), 3);
    }
    // data is dropped — if the handler held a reference, this would be UB

    // Second dispatch — completely different borrow
    {
        let other = [10u8, 20, 30, 40, 50];
        boxed.run(&mut world, &other);
        assert_eq!(*world.resource::<u64>(), 5);
    }
}

#[test]
fn hrtb_pipeline_opaque_closure() {
    let mut world = build_world();
    let r = world.registry_mut();

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .then(|w: &mut World, len: usize| {
            *w.resource_mut::<u64>() = len as u64;
        }, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    let data = vec![1u8, 2, 3, 4];
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 4);
}

#[test]
fn hrtb_pipeline_tee() {
    let mut world = build_world();
    let r = world.registry_mut();

    fn store_len_u32(mut out: ResMut<u32>, len: &usize) { *out = *len as u32; }

    // Side arm observes &usize (nested HRTB: C: for<'a> ChainCall<&'a usize>)
    let side = DagArmStart::<usize>::new()
        .then(store_len_u32, r);

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(slice_len, r)
        .tee(side)
        .then(store_len, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    let data = vec![1u8, 2, 3, 4, 5];
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 5); // main path stored len
    assert_eq!(*world.resource::<u32>(), 5); // side arm also observed len
}

#[test]
fn hrtb_send_bound() {
    fn assert_send<T: Send>() {}
    assert_send::<Box<dyn for<'a> Handler<&'a [u8]>>>();
    assert_send::<Box<dyn for<'a> Handler<Message<'a>>>>();
}

#[test]
fn hrtb_pipeline_local() {
    let mut world = build_world();
    let r = world.registry_mut();

    // Local<u64> persists across dispatches — counts invocations
    fn count_and_store(mut count: Local<u64>, mut out: ResMut<u64>, data: &[u8]) {
        *count += 1;
        *out = data.len() as u64 * 100 + *count;
    }

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(count_and_store, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    let data = vec![1u8, 2, 3];
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 301); // len=3, count=1

    let data2 = vec![10u8, 20];
    boxed.run(&mut world, &data2);
    assert_eq!(*world.resource::<u64>(), 202); // len=2, count=2

    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 303); // len=3, count=3
}

#[test]
fn hrtb_pipeline_multi_param() {
    let mut wb = WorldBuilder::new();
    wb.register::<f64>(2.5);
    wb.register::<u64>(0);
    let mut world = wb.build();
    let r = world.registry_mut();

    fn scaled_store(factor: Res<f64>, mut out: ResMut<u64>, data: &[u8]) {
        *out = (data.len() as f64 * *factor) as u64;
    }

    let pipeline = PipelineStart::<&[u8]>::new()
        .then(scaled_store, r)
        .build();

    let mut boxed: Box<dyn for<'a> Handler<&'a [u8]>> = Box::new(pipeline);

    let data = vec![1u8, 2, 3, 4]; // len=4, factor=2.5 → 10
    boxed.run(&mut world, &data);
    assert_eq!(*world.resource::<u64>(), 10);
}
