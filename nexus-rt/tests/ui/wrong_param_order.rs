// Handler with event before resources — should fail IntoHandler
use nexus_rt::{IntoHandler, Res, WorldBuilder};

struct Config;

fn bad_order(event: u32, _config: Res<Config>) {
    let _ = event;
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(Config);
    let world = wb.build();
    let reg = world.registry();

    let _handler = bad_order.into_handler(&reg);
}
