//! Spike: Can closures with Res<T>/ResMut<T> params work?

use nexus_rt::{Handler, IntoHandler, Res, ResMut, WorldBuilder};

nexus_rt::new_resource!(Val(u64));
nexus_rt::new_resource!(Out(u64));

// Named function — known to work
fn named_handler(val: Res<Val>, mut out: ResMut<Out>, event: u64) {
    out.0 = val.0 + event;
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(Val(42));
    wb.register(Out(0));
    let mut world = wb.build();

    // ── Named function: WORKS ──
    {
        let mut h = named_handler.into_handler(world.registry());
        h.run(&mut world, 10);
        assert_eq!(world.resource::<Out>().0, 52);
        println!("Named function with Res+ResMut: OK");
    }

    // ── Closure with NO params (arity-0): WORKS ──
    {
        let mut h = (|event: u64| {
            std::hint::black_box(event);
        })
        .into_handler(world.registry());
        h.run(&mut world, 10);
        println!("Closure no params: OK");
    }

    // ── Closure WITH Res param: TEST ──
    {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut wb2 = WorldBuilder::new();
            wb2.register(Val(42));
            wb2.register(Out(0));
            let mut world2 = wb2.build();

            let mut h = (|val: Res<Val>, event: u64| {
                std::hint::black_box(val.0 + event);
            })
            .into_handler(world2.registry());
            h.run(&mut world2, 10);
        }));
        match result {
            Ok(()) => println!("Closure with Res<T>: OK (compiles!)"),
            Err(_) => println!("Closure with Res<T>: PANICKED"),
        }
    }

    // ── Closure WITH Res+ResMut params: TEST ──
    {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut wb2 = WorldBuilder::new();
            wb2.register(Val(42));
            wb2.register(Out(0));
            let mut world2 = wb2.build();

            let mut h = (|val: Res<Val>, mut out: ResMut<Out>, event: u64| {
                out.0 = val.0 + event;
            })
            .into_handler(world2.registry());
            h.run(&mut world2, 10);
        }));
        match result {
            Ok(()) => println!("Closure with Res+ResMut: OK (compiles!)"),
            Err(_) => println!("Closure with Res+ResMut: PANICKED"),
        }
    }

    // ── FnOnce(&mut World): ALWAYS WORKS ──
    {
        let f = |world: &mut nexus_rt::World| {
            let v = world.resource::<Val>().0;
            world.resource_mut::<Out>().0 = v * 2;
        };
        f(&mut world);
        assert_eq!(world.resource::<Out>().0, 84);
        println!("FnOnce(&mut World): OK");
    }
}
