// Mistake: callback function missing &mut Ctx first param.
// Fix: add &mut MyCtx as the first parameter.

use nexus_rt::{IntoCallback, WorldBuilder};

struct MyCtx;

// Missing &mut MyCtx — should be fn(&mut MyCtx, u32)
fn bad_callback(event: u32) -> u32 {
    event
}

fn register_callback<C, E, P>(f: impl IntoCallback<C, E, P>, ctx: C, registry: &nexus_rt::Registry) {
    let _cb = f.into_callback(ctx, registry);
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    register_callback::<MyCtx, u32, _>(bad_callback, MyCtx, &reg);
}
