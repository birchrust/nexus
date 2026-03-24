// Mistake: scan step without &mut Acc as first param.
// Fix: first param must be &mut Accumulator.

use nexus_rt::{PipelineBuilder, WorldBuilder};

// Missing &mut acc — should be fn(&mut u64, u32) -> u32
fn bad_scan(x: u32) -> u32 {
    x + 1
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _ = PipelineBuilder::<u32>::new()
        .scan::<u64, _, _, _>(0u64, bad_scan, &reg);
}
