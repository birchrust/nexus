// Mistake: handler with >8 resource parameters (beyond macro arity).
// Fix: consolidate resources into fewer structs.

use nexus_rt::{IntoHandler, Res, Resource, WorldBuilder};

#[derive(Resource)] struct R0;
#[derive(Resource)] struct R1;
#[derive(Resource)] struct R2;
#[derive(Resource)] struct R3;
#[derive(Resource)] struct R4;
#[derive(Resource)] struct R5;
#[derive(Resource)] struct R6;
#[derive(Resource)] struct R7;
#[derive(Resource)] struct R8;

fn too_many(
    _r0: Res<R0>, _r1: Res<R1>, _r2: Res<R2>, _r3: Res<R3>,
    _r4: Res<R4>, _r5: Res<R5>, _r6: Res<R6>, _r7: Res<R7>,
    _r8: Res<R8>,
    _event: u32,
) {
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(R0); wb.register(R1); wb.register(R2); wb.register(R3);
    wb.register(R4); wb.register(R5); wb.register(R6); wb.register(R7);
    wb.register(R8);
    let world = wb.build();
    let reg = world.registry();

    let _handler = too_many.into_handler(&reg);
}
