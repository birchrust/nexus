// Mistake: field type that doesn't implement Param
// Fix: use Res<T>, ResMut<T>, Local<T>, Option<Res<T>>, or another #[derive(Param)] struct

use nexus_rt::Param;

#[derive(Param)]
struct BadField<'w> {
    x: &'w u32,  // &u32 doesn't impl Param
}

fn main() {}
