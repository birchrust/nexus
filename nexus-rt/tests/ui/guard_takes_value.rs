// guard with fn(T) -> bool instead of fn(&T) -> bool
use nexus_rt::{PipelineBuilder, WorldBuilder};

fn check_value(x: u32) -> bool {
    x > 10
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    // guard expects fn(&u32) -> bool, not fn(u32) -> bool
    let _ = PipelineBuilder::<u32>::new().guard(check_value, &reg);
}
