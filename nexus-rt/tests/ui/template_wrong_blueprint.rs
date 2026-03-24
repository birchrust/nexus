// Mistake: function signature doesn't match Blueprint's Event/Params types.
// Fix: function params must match Blueprint::Params then Blueprint::Event.

use nexus_rt::{HandlerTemplate, Res, WorldBuilder, handler_blueprint};

struct Config;

handler_blueprint!(MyBlueprint, Event = u32, Params = (Res<Config>,));

// Wrong: takes String instead of u32
fn wrong_event(_config: Res<Config>, _event: String) {
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(Config);
    let world = wb.build();
    let reg = world.registry();

    let _template = HandlerTemplate::<MyBlueprint>::new(wrong_event, &reg);
}
