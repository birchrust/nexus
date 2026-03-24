// Mistake: handler with event before resources.
// Fix: resources first, event last.

use nexus_rt::{IntoHandler, Res, WorldBuilder};

struct Config;

fn bad_order(event: u32, _config: Res<Config>) {
    let _ = event;
}

// Helper that requires IntoHandler bound — triggers E0277 with our diagnostic
fn register_handler<E, P>(f: impl IntoHandler<E, P>, registry: &nexus_rt::Registry) {
    let _h = f.into_handler(registry);
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(Config);
    let world = wb.build();
    let reg = world.registry();

    register_handler::<u32, _>(bad_order, &reg);
}
