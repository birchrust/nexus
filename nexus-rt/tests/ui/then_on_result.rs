// .then() on Result output — should suggest .map() or .catch()
use nexus_rt::{PipelineBuilder, WorldBuilder};

fn to_result(x: u32) -> Result<u32, String> {
    Ok(x)
}
fn takes_u32(x: u32) -> u32 {
    x + 1
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _ = PipelineBuilder::<u32>::new()
        .then(to_result, &reg) // output is Result<u32, String>
        .then(takes_u32, &reg); // ERROR: takes_u32 expects u32, not Result
}
