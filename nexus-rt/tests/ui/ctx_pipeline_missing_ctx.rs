// Mistake: ctx pipeline step missing &mut C first param.
// Fix: context-aware steps must take &mut C as the first parameter.

use nexus_rt::{CtxPipelineBuilder, Res, WorldBuilder};

struct MyCtx;

nexus_rt::new_resource!(Val(u64));

// Missing &mut MyCtx — this is a regular pipeline step, not a ctx step
fn bad_step(val: Res<Val>, input: u32) -> u64 {
    val.0 + input as u64
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(Val(0));
    let world = wb.build();
    let reg = world.registry();

    let _pipeline = CtxPipelineBuilder::<MyCtx, u32>::new().then(bad_step, reg);
}
