// Mistake: guard step takes T by value instead of &T.
// Fix: change fn(u32) -> bool to fn(&u32) -> bool.
// Guard/filter/tap/inspect pass the value through —
// the step only observes it.

use nexus_rt::{PipelineBuilder, WorldBuilder};

fn to_option(x: u32) -> Option<u32> {
    Some(x)
}

fn check_value(x: u32) -> bool {
    x > 10
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    // guard on Option<u32> expects fn(&u32) -> bool, not fn(u32) -> bool
    let _ = PipelineBuilder::<u32>::new()
        .then(to_option, &reg)
        .guard(check_value, &reg);
}
