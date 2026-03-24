// Mistake: merge function args don't match fork arm output types.
// Fix: merge function must accept references matching each arm's output.

use nexus_rt::{DagBuilder, WorldBuilder};

fn identity(x: u32) -> u32 { x }
fn to_string(x: u32) -> String { x.to_string() }

// Merge expects (&u32, &String) but this takes (&u32, &u32)
fn bad_merge(a: &u32, b: &u32) -> u64 {
    (*a + *b) as u64
}

fn main() {
    let mut wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _ = DagBuilder::<u32>::new()
        .root(identity, &reg)
        .fork()
        .arm(|seed| seed.then(identity, &reg))
        .arm(|seed| seed.then(to_string, &reg))
        .merge(bad_merge, &reg);
}
