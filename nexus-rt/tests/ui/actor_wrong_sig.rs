// Mistake: actor step function missing &mut Ctx first param.
// Fix: add &mut MyCtx as the first parameter.

use nexus_rt::{IntoActor, WorldBuilder};

struct MyCtx;

// Missing &mut MyCtx — should be fn(&mut MyCtx, ...)
fn bad_step(val: u32) {
    let _ = val;
}

fn main() {
    let wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _actor = bad_step.into_actor(MyCtx, reg);
}
