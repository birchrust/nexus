// Mistake: #[derive(View)] on an enum.
// Fix: View can only be derived for structs with named fields.

use nexus_rt::View;

#[derive(View)]
#[source(u32)]
enum BadView {
    A,
    B(u32),
}

fn main() {}
