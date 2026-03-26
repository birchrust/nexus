// Mistake: using .view::<V>() where V doesn't impl View for the event type.
// Fix: implement View<MyEvent> for the view marker.

use nexus_rt::{PipelineBuilder, View};

struct OrderView {
    qty: u64,
}

struct AsOrderView;

// AsOrderView impls View<GoodEvent> but NOT View<BadEvent>
struct GoodEvent {
    qty: u64,
}

struct BadEvent {
    name: String,
}

impl View<GoodEvent> for AsOrderView {
    type ViewType<'a> = OrderView;
    type StaticViewType = OrderView;
    fn view(source: &GoodEvent) -> OrderView {
        OrderView { qty: source.qty }
    }
}

fn main() {
    // This should fail: AsOrderView doesn't impl View<BadEvent>
    let _ = PipelineBuilder::<BadEvent>::new()
        .view::<AsOrderView>();
}
