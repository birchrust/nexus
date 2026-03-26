// Mistake: trying to use .then() inside a view scope.
// Fix: .then() transforms the event and needs ownership. Use .tap() or
// .guard() inside a view scope, or close with .end_view() first.

use nexus_rt::{PipelineBuilder, View};

struct OrderView {
    qty: u64,
}

struct AsOrderView;
unsafe impl View<u32> for AsOrderView {
    type ViewType<'a> = OrderView;
    type StaticViewType = OrderView;
    fn view(_s: &u32) -> OrderView {
        OrderView { qty: 0 }
    }
}

fn transform(_v: u32) -> u64 {
    0
}

fn main() {
    // Should fail: .then() is not available on ViewScope
    let _ = PipelineBuilder::<u32>::new()
        .view::<AsOrderView>()
        .then(transform);
}
