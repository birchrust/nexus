// IntoCallback with missing context param
use nexus_rt::{IntoCallback, WorldBuilder};

struct MyCtx;

// Missing &mut MyCtx as first param — should fail IntoCallback
fn bad_callback(event: u32) -> u32 {
    event
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _cb = bad_callback.into_callback(MyCtx, &reg);
}
