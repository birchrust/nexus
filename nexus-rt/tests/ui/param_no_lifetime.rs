// Mistake: derive(Param) on a struct with no lifetime parameter
// Fix: add a lifetime parameter, e.g., `struct MyParam<'w>`

use nexus_rt::Param;

#[derive(Param)]
struct NoLifetime {
    x: u32,
}

fn main() {}
