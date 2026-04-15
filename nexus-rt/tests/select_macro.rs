//! Integration tests for the `select!` macro.

use nexus_rt::{Handler, PipelineBuilder, WorldBuilder, select};

// =============================================================================
// Test enum
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    A,
    B,
    C,
}

// =============================================================================
// Tier 1 — input is the match value directly
// =============================================================================

fn handle_a(v: Kind) {
    assert_eq!(v, Kind::A);
}

fn handle_b(v: Kind) {
    assert_eq!(v, Kind::B);
}

fn handle_c(v: Kind) {
    assert_eq!(v, Kind::C);
}

#[test]
fn select_tier1_basic() {
    let mut world = WorldBuilder::new().build();
    let reg = world.registry();

    let mut pipeline = PipelineBuilder::<Kind>::new()
        .then(
            select! {
                reg,
                Kind::A => handle_a,
                Kind::B => handle_b,
                Kind::C => handle_c,
            },
            reg,
        )
        .build();

    pipeline.run(&mut world, Kind::A);
    pipeline.run(&mut world, Kind::B);
    pipeline.run(&mut world, Kind::C);
}

// =============================================================================
// Tier 1 — default arm
// =============================================================================

#[test]
fn select_tier1_default() {
    let mut world = WorldBuilder::new().build();
    let reg = world.registry();

    let mut pipeline = PipelineBuilder::<Kind>::new()
        .then(
            select! {
                reg,
                Kind::A => handle_a,
                _ => |_w, _x| { /* default — no-op */ },
            },
            reg,
        )
        .build();

    // All variants handled without panic.
    pipeline.run(&mut world, Kind::A);
    pipeline.run(&mut world, Kind::B);
    pipeline.run(&mut world, Kind::C);
}

// =============================================================================
// Tier 2 — match on a field, arms take the struct
// =============================================================================

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
struct Order {
    kind: Kind,
    id: u64,
}

fn handle_order_a(o: Order) {
    assert_eq!(o.kind, Kind::A);
}

fn handle_order_b(o: Order) {
    assert_eq!(o.kind, Kind::B);
}

#[test]
fn select_tier2_key() {
    let mut world = WorldBuilder::new().build();
    let reg = world.registry();

    let mut pipeline = PipelineBuilder::<Order>::new()
        .then(
            select! {
                reg,
                key: |o: &Order| o.kind,
                Kind::A => handle_order_a,
                Kind::B => handle_order_b,
                Kind::C => |_o: Order| {},
            },
            reg,
        )
        .build();

    pipeline.run(
        &mut world,
        Order {
            kind: Kind::A,
            id: 1,
        },
    );
    pipeline.run(
        &mut world,
        Order {
            kind: Kind::B,
            id: 2,
        },
    );
    pipeline.run(
        &mut world,
        Order {
            kind: Kind::C,
            id: 3,
        },
    ); // Kind::C reuses handle_order_a — just verifies dispatch, not kind assertion
}

// =============================================================================
// Tier 3 — key + project
// =============================================================================

fn process_id(id: u64) {
    assert!(id > 0);
}

#[test]
fn select_tier3_key_project() {
    let mut world = WorldBuilder::new().build();
    let reg = world.registry();

    // Input is (u64, Kind). Match on Kind, arms receive u64.
    let mut pipeline = PipelineBuilder::<(u64, Kind)>::new()
        .then(
            select! {
                reg,
                key:     |(_, k): &(u64, Kind)| *k,
                project: |(id, _)| id,
                Kind::A => process_id,
                Kind::B => process_id,
                Kind::C => process_id,
            },
            reg,
        )
        .build();

    pipeline.run(&mut world, (42, Kind::A));
    pipeline.run(&mut world, (99, Kind::B));
    pipeline.run(&mut world, (7, Kind::C));
}

// =============================================================================
// Tier 3 — with default arm
// =============================================================================

#[test]
fn select_tier3_default() {
    let mut world = WorldBuilder::new().build();
    let reg = world.registry();

    let mut pipeline = PipelineBuilder::<(u64, Kind)>::new()
        .then(
            select! {
                reg,
                key:     |(_, k): &(u64, Kind)| *k,
                project: |(id, _)| id,
                Kind::A => process_id,
                _ => |_w, _input| { /* default */ },
            },
            reg,
        )
        .build();

    pipeline.run(&mut world, (42, Kind::A));
    pipeline.run(&mut world, (42, Kind::B));
}

// =============================================================================
// Callback form
// =============================================================================

struct Ctx {
    count: u32,
}

fn on_a(ctx: &mut Ctx, _kind: Kind) {
    ctx.count += 1;
}

fn on_b(ctx: &mut Ctx, _kind: Kind) {
    ctx.count += 10;
}

#[test]
fn select_callback_basic() {
    use nexus_rt::CtxPipelineBuilder;

    let mut world = WorldBuilder::new().build();
    let reg = world.registry();

    let mut pipeline = CtxPipelineBuilder::<Ctx, Kind>::new()
        .then(
            select! {
                reg,
                ctx: Ctx,
                Kind::A => on_a,
                Kind::B => on_b,
                Kind::C => on_a,
            },
            reg,
        )
        .build();

    let mut ctx = Ctx { count: 0 };
    pipeline.run(&mut ctx, &mut world, Kind::A);
    assert_eq!(ctx.count, 1);
    pipeline.run(&mut ctx, &mut world, Kind::B);
    assert_eq!(ctx.count, 11);
    pipeline.run(&mut ctx, &mut world, Kind::C);
    assert_eq!(ctx.count, 12);
}

#[test]
fn select_callback_mutates_ctx() {
    use nexus_rt::CtxPipelineBuilder;

    fn increment(ctx: &mut Ctx, _kind: Kind) {
        ctx.count += 1;
    }

    fn double_increment(ctx: &mut Ctx, _kind: Kind) {
        ctx.count += 2;
    }

    let mut world = WorldBuilder::new().build();
    let reg = world.registry();

    let mut pipeline = CtxPipelineBuilder::<Ctx, Kind>::new()
        .then(
            select! {
                reg,
                ctx: Ctx,
                Kind::A => increment,
                Kind::B => double_increment,
                Kind::C => increment,
            },
            reg,
        )
        .build();

    let mut ctx = Ctx { count: 0 };
    pipeline.run(&mut ctx, &mut world, Kind::A);
    assert_eq!(ctx.count, 1);
    pipeline.run(&mut ctx, &mut world, Kind::B);
    assert_eq!(ctx.count, 3);
    pipeline.run(&mut ctx, &mut world, Kind::C);
    assert_eq!(ctx.count, 4);
}
