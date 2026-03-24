// Mistake: guard takes T by value instead of &T.
// Fix: change fn(u32) -> bool to fn(&u32) -> bool.

use nexus_rt::{PipelineBuilder, WorldBuilder};

fn identity(x: u32) -> u32 {
    x
}

fn check_value(x: u32) -> bool {
    x > 10
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    // guard expects fn(&u32) -> bool, not fn(u32) -> bool
    let _ = PipelineBuilder::<u32>::new()
        .then(identity, &reg)
        .guard(check_value, &reg);
}
