// Mistake: registering a raw type without #[derive(Resource)]
// Fix: add #[derive(Resource)] to your type, or use new_resource!

fn main() {
    let mut wb = nexus_rt::WorldBuilder::new();
    wb.register::<u64>(42);
}
