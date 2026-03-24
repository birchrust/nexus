// Mistake: derive(Param) on a struct with two lifetime parameters
// Fix: use exactly one lifetime parameter

use nexus_rt::Param;

#[derive(Param)]
struct TwoLifetimes<'a, 'b> {
    x: &'a u32,
    y: &'b u32,
}

fn main() {}
