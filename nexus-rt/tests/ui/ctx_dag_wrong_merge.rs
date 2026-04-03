// Mistake: ctx DAG merge function has wrong number of arm inputs.
// Fix: merge function must take references to each arm's output.

use nexus_rt::{CtxDagBuilder, WorldBuilder};

struct MyCtx;

fn root(_ctx: &mut MyCtx, x: u32) -> u64 {
    x as u64
}

fn arm_a(_ctx: &mut MyCtx, val: &u64) -> u32 {
    *val as u32
}

fn arm_b(_ctx: &mut MyCtx, val: &u64) -> String {
    val.to_string()
}

// Wrong: takes values, not references. Merge steps take (&A, &B).
fn bad_merge(_ctx: &mut MyCtx, a: u32, b: String) {
    let _ = (a, b);
}

fn main() {
    let wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _dag = CtxDagBuilder::<MyCtx, u32>::new()
        .root(root, reg)
        .fork()
        .arm(|seed| seed.then(arm_a, reg))
        .arm(|seed| seed.then(arm_b, reg))
        .merge(bad_merge, reg);
}
