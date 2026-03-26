// Mistake: callback function missing &mut Ctx first param.
// Fix: add &mut MyCtx as the first parameter.

use nexus_rt::{IntoCallback, Res, Resource, WorldBuilder};

struct MyCtx;

#[derive(Resource)]
struct Config;

// Missing &mut MyCtx — should be fn(&mut MyCtx, Res<Config>, u32)
fn bad_callback(_config: Res<Config>, _event: u32) {
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(Config);
    let world = wb.build();
    let reg = world.registry();

    let _cb = bad_callback.into_callback(MyCtx, &reg);
}
