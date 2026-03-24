// .then() on Option output — should suggest .map()
use nexus_rt::{PipelineBuilder, WorldBuilder};

fn to_option(x: u32) -> Option<u32> {
    Some(x)
}
fn takes_u32(x: u32) -> u32 {
    x + 1
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _ = PipelineBuilder::<u32>::new()
        .then(to_option, &reg) // output is Option<u32>
        .then(takes_u32, &reg); // ERROR: takes_u32 expects u32, not Option<u32>
}
