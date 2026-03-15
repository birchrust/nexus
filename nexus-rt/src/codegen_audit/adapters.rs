//! Adapter, combinator, handler, and template codegen audit cases.
//!
//! Categories 23-25 from the assembly audit plan.

#![allow(clippy::type_complexity)]
#![allow(unused_variables)]

use super::helpers::*;
use crate::adapt::{Adapt, ByRef, Cloned, Owned};
use crate::catch_unwind::CatchAssertUnwindSafe;
use crate::template::{CallbackTemplate, HandlerTemplate};
use crate::{
    Broadcast, Handler, IntoCallback, IntoHandler, Res, ResMut, World, callback_blueprint, fan_out,
    handler_blueprint,
};

// ═══════════════════════════════════════════════════════════════════
// 23. Adapters
// ═══════════════════════════════════════════════════════════════════

// -- ByRef: owned → borrowed dispatch --

fn ref_handler_add(a: Res<ResA>, x: &u64) {
    let _ = a.0.wrapping_add(*x);
}

#[inline(never)]
pub fn adapt_by_ref(world: &mut World, input: u64) {
    let reg = world.registry();
    let inner = ref_handler_add.into_handler(&reg);
    let mut h = ByRef(inner);
    h.run(world, input);
}

// -- Cloned: borrowed → owned dispatch --

fn owned_handler_add(mut a: ResMut<ResA>, x: u64) {
    a.0 = a.0.wrapping_add(x);
}

#[inline(never)]
pub fn adapt_cloned(world: &mut World, input: &u64) {
    let reg = world.registry();
    let inner = owned_handler_add.into_handler(&reg);
    let mut h = Cloned(inner);
    h.run(world, input);
}

// -- Adapt: decode adapter --

#[inline(never)]
pub fn adapt_decode(world: &mut World, input: u64) {
    let reg = world.registry();
    let inner = owned_handler_add.into_handler(&reg);
    let mut h = Adapt::new(|wire: u64| if wire > 0 { Some(wire) } else { None }, inner);
    h.run(world, input);
}

#[inline(never)]
pub fn adapt_decode_skip(world: &mut World, input: u64) {
    let reg = world.registry();
    let inner = owned_handler_add.into_handler(&reg);
    let mut h = Adapt::new(
        |wire: u64| {
            if wire < 100 {
                Some(wire.wrapping_mul(2))
            } else {
                None
            }
        },
        inner,
    );
    // Half the inputs will be skipped — interesting for branch prediction codegen.
    h.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 24. Combinators (FanOut, Broadcast)
// ═══════════════════════════════════════════════════════════════════

#[inline(never)]
pub fn fanout_2(world: &mut World, input: u64) {
    let reg = world.registry();
    let h1 = ref_handler_add.into_handler(&reg);
    let h2 = ref_handler_add.into_handler(&reg);
    let mut fan = fan_out!(h1, h2);
    fan.run(world, input);
}

#[inline(never)]
pub fn fanout_4(world: &mut World, input: u64) {
    let reg = world.registry();
    let h1 = ref_handler_add.into_handler(&reg);
    let h2 = ref_handler_add.into_handler(&reg);
    let h3 = ref_handler_add.into_handler(&reg);
    let h4 = ref_handler_add.into_handler(&reg);
    let mut fan = fan_out!(h1, h2, h3, h4);
    fan.run(world, input);
}

#[inline(never)]
pub fn broadcast_2(world: &mut World, input: u64) {
    let reg = world.registry();
    let h1 = ref_handler_add.into_handler(&reg);
    let h2 = ref_handler_add.into_handler(&reg);
    let mut bc = Broadcast::<u64>::new();
    bc.add(h1);
    bc.add(h2);
    bc.run(world, input);
}

#[inline(never)]
pub fn broadcast_4(world: &mut World, input: u64) {
    let reg = world.registry();
    let h1 = ref_handler_add.into_handler(&reg);
    let h2 = ref_handler_add.into_handler(&reg);
    let h3 = ref_handler_add.into_handler(&reg);
    let h4 = ref_handler_add.into_handler(&reg);
    let mut bc = Broadcast::<u64>::new();
    bc.add(h1);
    bc.add(h2);
    bc.add(h3);
    bc.add(h4);
    bc.run(world, input);
}

// ═══════════════════════════════════════════════════════════════════
// 25. Handlers & templates
// ═══════════════════════════════════════════════════════════════════

// -- Direct IntoHandler dispatch --

fn handler_1_param(a: Res<ResA>, x: u64) {
    let _ = a.0.wrapping_add(x);
}
fn handler_2_param(a: Res<ResA>, b: Res<ResB>, x: u64) {
    let _ = a.0.wrapping_add(b.0 as u64).wrapping_add(x);
}
fn handler_3_param(a: Res<ResA>, b: Res<ResB>, c: Res<ResC>, x: u64) {
    let _ = a.0.wrapping_add(b.0 as u64).wrapping_add(c.0 as u64);
}

#[inline(never)]
pub fn handler_0_param_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut h = owned_handler_add.into_handler(&reg);
    h.run(world, input);
}

#[inline(never)]
pub fn handler_1_param_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut h = handler_1_param.into_handler(&reg);
    h.run(world, input);
}

#[inline(never)]
pub fn handler_2_param_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut h = handler_2_param.into_handler(&reg);
    h.run(world, input);
}

#[inline(never)]
pub fn handler_3_param_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut h = handler_3_param.into_handler(&reg);
    h.run(world, input);
}

// -- Virtual (boxed) dispatch --

#[inline(never)]
pub fn handler_virtual_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut h: Box<dyn Handler<u64>> = Box::new(owned_handler_add.into_handler(&reg));
    h.run(world, input);
}

// -- CatchAssertUnwindSafe --

#[inline(never)]
pub fn handler_catch_unwind(world: &mut World, input: u64) {
    let reg = world.registry();
    let inner = owned_handler_add.into_handler(&reg);
    let mut h = CatchAssertUnwindSafe::new(inner);
    h.run(world, input);
}

// -- HandlerTemplate: resolve once, stamp many --

handler_blueprint!(TickBlueprint, Event = u64, Params = (ResMut<'static, ResA>,));

fn tick_handler(mut a: ResMut<ResA>, x: u64) {
    a.0 = a.0.wrapping_add(x);
}

#[inline(never)]
pub fn template_generate_and_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let tmpl = HandlerTemplate::<TickBlueprint>::new(tick_handler, &reg);
    let mut h = tmpl.generate();
    h.run(world, input);
}

#[inline(never)]
pub fn template_stamp_5(world: &mut World, input: u64) {
    let reg = world.registry();
    let tmpl = HandlerTemplate::<TickBlueprint>::new(tick_handler, &reg);
    let mut h0 = tmpl.generate();
    let mut h1 = tmpl.generate();
    let mut h2 = tmpl.generate();
    let mut h3 = tmpl.generate();
    let mut h4 = tmpl.generate();
    h0.run(world, input);
    h1.run(world, input.wrapping_add(1));
    h2.run(world, input.wrapping_add(2));
    h3.run(world, input.wrapping_add(3));
    h4.run(world, input.wrapping_add(4));
}

// ═══════════════════════════════════════════════════════════════════
// Remaining gaps
// ═══════════════════════════════════════════════════════════════════

// ---- 23.3: Cloned with String (heap clone) ----

fn string_handler(mut a: ResMut<ResA>, x: String) {
    a.0 = x.len() as u64;
}

#[inline(never)]
pub fn adapt_cloned_string(world: &mut World, input: &String) {
    let reg = world.registry();
    let inner = string_handler.into_handler(&reg);
    let mut h = Cloned(inner);
    h.run(world, input);
}

// ---- 23.4: Owned adapter (&str → String) ----

#[inline(never)]
pub fn adapt_owned_str(world: &mut World, input: &str) {
    let reg = world.registry();
    let inner = string_handler.into_handler(&reg);
    let mut h = Owned::<_, str>::new(inner);
    h.run(world, input);
}

// ---- 23.5: nested adapter (Cloned(ByRef(inner))) ----

#[inline(never)]
pub fn adapt_nested(world: &mut World, input: &u64) {
    let reg = world.registry();
    let inner = ref_handler_add.into_handler(&reg);
    // Cloned: &u64 → u64 (copy), ByRef: u64 → &u64 → inner
    // Two-adapter round-trip: borrow → clone → reborrow
    let mut h = Cloned(ByRef(inner));
    h.run(world, input);
}

// ---- 23.8: fan_out with 8 handlers ----

#[inline(never)]
pub fn fanout_8(world: &mut World, input: u64) {
    let reg = world.registry();
    let h1 = ref_handler_add.into_handler(&reg);
    let h2 = ref_handler_add.into_handler(&reg);
    let h3 = ref_handler_add.into_handler(&reg);
    let h4 = ref_handler_add.into_handler(&reg);
    let h5 = ref_handler_add.into_handler(&reg);
    let h6 = ref_handler_add.into_handler(&reg);
    let h7 = ref_handler_add.into_handler(&reg);
    let h8 = ref_handler_add.into_handler(&reg);
    let mut fan = fan_out!(h1, h2, h3, h4, h5, h6, h7, h8);
    fan.run(world, input);
}

// ---- 23.9: broadcast with 3 handlers ----

#[inline(never)]
pub fn broadcast_3(world: &mut World, input: u64) {
    let reg = world.registry();
    let h1 = ref_handler_add.into_handler(&reg);
    let h2 = ref_handler_add.into_handler(&reg);
    let h3 = ref_handler_add.into_handler(&reg);
    let mut bc = Broadcast::<u64>::new();
    bc.add(h1);
    bc.add(h2);
    bc.add(h3);
    bc.run(world, input);
}

// ---- 23.10: adapters inside fan_out ----

#[inline(never)]
pub fn adapt_in_fanout(world: &mut World, input: u64) {
    let reg = world.registry();
    let h1 = ref_handler_add.into_handler(&reg);
    let h2 = owned_handler_add.into_handler(&reg);
    let h3 = ref_handler_add.into_handler(&reg);
    // Cloned: &u64 → u64 → owned_handler, plain ref handlers
    let mut fan = fan_out!(Cloned(h2), h1, h3);
    fan.run(world, input);
}

// ---- 24.4: Callback dispatch (context + params) ----

struct CounterCtx {
    count: u64,
}

fn callback_handler(ctx: &mut CounterCtx, mut a: ResMut<ResA>, x: u64) {
    ctx.count += 1;
    a.0 = a.0.wrapping_add(x).wrapping_add(ctx.count);
}

#[inline(never)]
pub fn callback_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let mut h = callback_handler.into_callback(CounterCtx { count: 0 }, &reg);
    h.run(world, input);
}

// ---- 24.7: CallbackTemplate dispatch ----

callback_blueprint!(CountBlueprint, Context = CounterCtx, Event = u64, Params = (ResMut<'static, ResA>,));

fn count_tick(ctx: &mut CounterCtx, mut a: ResMut<ResA>, x: u64) {
    ctx.count += 1;
    a.0 = a.0.wrapping_add(x).wrapping_add(ctx.count);
}

#[inline(never)]
pub fn callback_template_dispatch(world: &mut World, input: u64) {
    let reg = world.registry();
    let tmpl = CallbackTemplate::<CountBlueprint>::new(count_tick, &reg);
    let mut h = tmpl.generate(CounterCtx { count: 0 });
    h.run(world, input);
}
