// .map() on non-Option/non-Result pipeline — method doesn't exist
use nexus_rt::{PipelineBuilder, WorldBuilder};

fn add_one(x: u32) -> u32 {
    x + 1
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    // .map() only exists on Option/Result chains
    let _ = PipelineBuilder::<u32>::new()
        .then(add_one, &reg)
        .map(add_one, &reg); // ERROR: no method named `map`
}
