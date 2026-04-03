//! Actor system performance benchmark.
//!
//! Measures cycle counts for actor dispatch across configurations:
//! - mark + dispatch with varying actor counts
//! - per-actor dispatch cost (with pre-resolved Params)
//! - dedup pressure (many sources, overlapping subscriptions)
//! - source registry lookup cost
//!
//! ```bash
//! taskset -c 0 cargo run --release -p nexus-rt --example perf_actors
//! ```

use std::hint::black_box;

use nexus_notify::Token;
use nexus_rt::{
    ActorNotify, ActorSystem, DeferredRemovals, IntoActor, Res, ResMut, SourceRegistry,
    WorldBuilder,
};

// =============================================================================
// Bench infrastructure
// =============================================================================

const ITERATIONS: usize = 100_000;
const WARMUP: usize = 10_000;
const BATCH: u64 = 100;

nexus_rt::new_resource!(Val(u64));
nexus_rt::new_resource!(Out(u64));

#[inline(always)]
#[cfg(target_arch = "x86_64")]
fn rdtsc_start() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
#[cfg(target_arch = "x86_64")]
fn rdtsc_end() -> u64 {
    unsafe {
        let mut aux = 0u32;
        let tsc = core::arch::x86_64::__rdtscp(&raw mut aux);
        core::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn bench_batched<F: FnMut() -> u64>(name: &str, mut f: F) -> (u64, u64, u64) {
    for _ in 0..WARMUP {
        black_box(f());
    }
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            black_box(f());
        }
        let end = rdtsc_end();
        samples.push(end.wrapping_sub(start) / BATCH);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 50.0);
    let p99 = percentile(&samples, 99.0);
    let p999 = percentile(&samples, 99.9);
    println!("{:<56} {:>8} {:>8} {:>8}", name, p50, p99, p999);
    (p50, p99, p999)
}

fn print_header(title: &str) {
    println!("\n=== {} ===\n", title);
    println!(
        "{:<56} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(84));
}

// =============================================================================
// Actor step functions
// =============================================================================

struct Ctx {
    _actor_id: Token,
}

fn noop_step(_ctx: &mut Ctx) {
    // Pure dispatch overhead — no resource access
}

fn one_res_step(_ctx: &mut Ctx, _val: Res<Val>) {
    // One Param fetch
}

fn two_res_step(_ctx: &mut Ctx, _val: Res<Val>, mut out: ResMut<Out>) {
    out.0 += 1;
}

// =============================================================================
// Scenarios
// =============================================================================

fn scenario_dispatch_scaling() {
    print_header("mark + dispatch — varying actor counts (noop step)");

    for &count in &[1, 5, 10, 50, 200] {
        let mut wb = WorldBuilder::new();
        wb.register(ActorNotify::new(4, count + 16));
        wb.register(DeferredRemovals::default());
        let mut world = wb.build();
        let mut system = ActorSystem::new(&world);

        let src = world.resource_mut::<ActorNotify>().register_source();

        for _ in 0..count {
            let token = world.resource_mut::<ActorNotify>().alloc_actor();
            let actor = noop_step.into_actor(Ctx { _actor_id: token }, world.registry());
            world
                .resource_mut::<ActorNotify>()
                .insert(token, actor)
                .subscribe(src);
        }

        let label = format!("mark + dispatch ({} actors, noop)", count);
        bench_batched(&label, || {
            world.resource_mut::<ActorNotify>().mark(src);
            system.dispatch(&mut world);
            0
        });
    }
}

fn scenario_param_cost() {
    print_header("dispatch cost per Param arity (10 actors)");

    // Noop (0 params)
    {
        let mut wb = WorldBuilder::new();
        wb.register(ActorNotify::new(4, 16));
        wb.register(DeferredRemovals::default());
        let mut world = wb.build();
        let mut system = ActorSystem::new(&world);
        let src = world.resource_mut::<ActorNotify>().register_source();

        for _ in 0..10 {
            let token = world.resource_mut::<ActorNotify>().alloc_actor();
            let actor = noop_step.into_actor(Ctx { _actor_id: token }, world.registry());
            world
                .resource_mut::<ActorNotify>()
                .insert(token, actor)
                .subscribe(src);
        }

        bench_batched("10 actors × 0 params (noop)", || {
            world.resource_mut::<ActorNotify>().mark(src);
            system.dispatch(&mut world);
            0
        });
    }

    // 1 Param (Res<Val>)
    {
        let mut wb = WorldBuilder::new();
        wb.register(Val(42));
        wb.register(ActorNotify::new(4, 16));
        wb.register(DeferredRemovals::default());
        let mut world = wb.build();
        let mut system = ActorSystem::new(&world);
        let src = world.resource_mut::<ActorNotify>().register_source();

        for _ in 0..10 {
            let token = world.resource_mut::<ActorNotify>().alloc_actor();
            let actor = one_res_step.into_actor(Ctx { _actor_id: token }, world.registry());
            world
                .resource_mut::<ActorNotify>()
                .insert(token, actor)
                .subscribe(src);
        }

        bench_batched("10 actors × 1 param (Res<Val>)", || {
            world.resource_mut::<ActorNotify>().mark(src);
            system.dispatch(&mut world);
            0
        });
    }

    // 2 Params (Res<Val>, ResMut<Out>)
    {
        let mut wb = WorldBuilder::new();
        wb.register(Val(42));
        wb.register(Out(0));
        wb.register(ActorNotify::new(4, 16));
        wb.register(DeferredRemovals::default());
        let mut world = wb.build();
        let mut system = ActorSystem::new(&world);
        let src = world.resource_mut::<ActorNotify>().register_source();

        for _ in 0..10 {
            let token = world.resource_mut::<ActorNotify>().alloc_actor();
            let actor = two_res_step.into_actor(Ctx { _actor_id: token }, world.registry());
            world
                .resource_mut::<ActorNotify>()
                .insert(token, actor)
                .subscribe(src);
        }

        bench_batched("10 actors × 2 params (Res + ResMut)", || {
            world.resource_mut::<ActorNotify>().mark(src);
            system.dispatch(&mut world);
            0
        });
    }
}

fn scenario_dedup() {
    print_header("dedup — 50 actors × 10 sources (all subscribed to all)");

    let mut wb = WorldBuilder::new();
    wb.register(ActorNotify::new(16, 64));
    wb.register(DeferredRemovals::default());
    let mut world = wb.build();
    let mut system = ActorSystem::new(&world);

    let mut sources = Vec::new();
    for _ in 0..10 {
        sources.push(world.resource_mut::<ActorNotify>().register_source());
    }

    for _ in 0..50 {
        let token = world.resource_mut::<ActorNotify>().alloc_actor();
        let actor = noop_step.into_actor(Ctx { _actor_id: token }, world.registry());
        let mut reg = world.resource_mut::<ActorNotify>().insert(token, actor);
        for &src in &sources {
            reg = reg.subscribe(src);
        }
    }

    bench_batched("mark 10 sources + dispatch 50 actors (dedup)", || {
        let notify = world.resource_mut::<ActorNotify>();
        for &src in &sources {
            notify.mark(src);
        }
        system.dispatch(&mut world);
        0
    });
}

fn scenario_source_registry() {
    print_header("SourceRegistry lookup (cold path)");

    let mut registry = SourceRegistry::new();

    // Populate with 100 entries
    for i in 0..100u64 {
        registry.insert(i, nexus_rt::DataSource(i as usize));
    }

    bench_batched("get() — u64 key, 100 entries", || {
        let result = registry.get(&42u64);
        black_box(result);
        0
    });

    // Tuple keys
    let mut registry2 = SourceRegistry::new();
    for i in 0..100 {
        registry2.insert(("BTC", i), nexus_rt::DataSource(i));
    }

    bench_batched("get() — (&str, usize) key, 100 entries", || {
        let result = registry2.get(&("BTC", 42));
        black_box(result);
        0
    });
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    println!("Actor System Performance Benchmark");
    println!("Cycles per operation (batched, {} ops/sample)\n", BATCH);

    scenario_dispatch_scaling();
    scenario_param_cost();
    scenario_dedup();
    scenario_source_registry();
}
