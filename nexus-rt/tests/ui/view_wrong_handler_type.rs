// Mistake: handler inside view scope takes wrong view type.
// Fix: the handler's last parameter must be &ViewType.

use nexus_rt::{PipelineBuilder, View, WorldBuilder};

struct OrderView {
    qty: u64,
}

struct WrongView {
    name: String,
}

struct AsOrderView;
unsafe impl View<u32> for AsOrderView {
    type ViewType<'a> = OrderView;
    type StaticViewType = OrderView;
    fn view(_s: &u32) -> OrderView {
        OrderView { qty: 0 }
    }
}

// This handler takes &WrongView, but the view scope produces &OrderView
fn bad_handler(_v: &WrongView) {}

fn main() {
    let wb = WorldBuilder::new();
    let world = wb.build();
    let reg = world.registry();

    let _ = PipelineBuilder::<u32>::new()
        .view::<AsOrderView>()
        .tap(bad_handler, reg);
}
