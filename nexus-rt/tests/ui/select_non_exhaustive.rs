//! Verifies that `select!` preserves rustc exhaustiveness checking.
//!
//! Omitting an enum variant without a `_ =>` default arm must produce
//! a "non-exhaustive patterns" error pointing at the missing variant.
//! This is the central guarantee of the macro: the expansion is a
//! literal `match`, so rustc enforces exhaustiveness on the user's
//! patterns the same way it would for a hand-written match.

use nexus_rt::{PipelineBuilder, WorldBuilder, select};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    A,
    B,
    C,
}

fn handle(_v: Kind) {}

fn main() {
    let world = WorldBuilder::new().build();
    let reg = world.registry();

    // Missing Kind::C, no default arm — must fail with "non-exhaustive patterns".
    let _pipeline = PipelineBuilder::<Kind>::new()
        .then(
            select! {
                reg,
                Kind::A => handle,
                Kind::B => handle,
            },
            reg,
        )
        .build();
}
