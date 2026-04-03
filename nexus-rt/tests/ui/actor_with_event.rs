// Mistake: actor step function takes an event argument.
// Fix: actors don't receive events — use Handler<E> instead,
// or remove the event parameter.

use nexus_rt::{IntoActor, Res, WorldBuilder};

struct MyCtx;

nexus_rt::new_resource!(Val(u64));

// This is a Handler<u32> signature, not an Actor signature.
// Actor steps are: fn(&mut C, Params...)
// Handler steps are: fn(&mut C, Params..., Event)
fn bad_step(ctx: &mut MyCtx, val: Res<Val>, event: u32) {
    let _ = (ctx, val, event);
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(Val(0));
    let world = wb.build();
    let reg = world.registry();

    let _actor = bad_step.into_actor(MyCtx, reg);
}
