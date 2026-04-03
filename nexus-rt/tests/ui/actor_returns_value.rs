// Mistake: actor step function returns a value.
// Fix: actor steps return nothing — remove the return type.

use nexus_rt::{IntoActor, WorldBuilder};

struct MyCtx;

// Actor steps must return () — returning u32 is wrong
fn bad_step(ctx: &mut MyCtx) -> u32 {
    let _ = ctx;
    42
}

fn main() {
    let wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _actor = bad_step.into_actor(MyCtx, reg);
}
