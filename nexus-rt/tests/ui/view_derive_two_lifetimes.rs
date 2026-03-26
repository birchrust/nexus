// Mistake: #[derive(View)] with two lifetime parameters.
// Fix: view structs support at most one lifetime parameter.

use nexus_rt::View;

struct MyEvent {
    a: String,
    b: String,
}

#[derive(View)]
#[source(MyEvent)]
struct BadView<'a, 'b> {
    #[borrow]
    a: &'a str,
    #[borrow]
    b: &'b str,
}

fn main() {}
