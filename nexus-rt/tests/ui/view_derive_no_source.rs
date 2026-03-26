// Mistake: #[derive(View)] without any #[source] attribute.
// Fix: add at least one #[source(EventType)] to specify what event
// types this view can be constructed from.

use nexus_rt::View;

#[derive(View)]
struct OrderView<'a> {
    #[borrow]
    symbol: &'a str,
    qty: u64,
}

fn main() {}
