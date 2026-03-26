// Mistake: view has a field that doesn't exist on the source.
// Fix: either rename the view field to match, or use #[source(Type, from = "actual_name")].

use nexus_rt::View;

struct MyEvent {
    symbol: String,
    quantity: u64,  // note: "quantity", not "qty"
}

#[derive(View)]
#[source(MyEvent)]
struct BadView {
    qty: u64,  // MyEvent doesn't have a field named "qty"
}

fn main() {}
