#![allow(clippy::type_complexity)]
// Pipeline view scopes — borrow a projected view from an event.
//
// `.view::<V>()` opens a scope where steps operate on a read-only
// view constructed from the event. `.end_view()` closes the scope
// and the original event continues unchanged.
//
// Lifetime erasure: the view may borrow from the event (e.g.,
// `OrderView<'a>` with `symbol: &'a str`). IntoRefStep resolves
// against `StaticViewType` ('static stand-in). `with_view` bridges
// the two via a scoped transmute — same pattern as std::thread::scope.

use core::marker::PhantomData;

use crate::pipeline::{ChainCall, IntoRefStep, PipelineChain, RefStepCall};
use crate::world::World;

// =============================================================================
// View trait
// =============================================================================

/// Associates a source type with a projected view via a marker.
///
/// `ViewType<'a>` is the real view with borrowed fields.
/// `StaticViewType` is the same struct with `'static` — used only for
/// step resolution at build time, never observed at runtime.
///
/// # Examples
///
/// ```
/// use nexus_rt::View;
///
/// struct OrderView<'a> { symbol: &'a str, qty: u64 }
/// struct NewOrder { symbol: String, qty: u64 }
///
/// struct AsOrderView;
/// impl View<NewOrder> for AsOrderView {
///     type ViewType<'a> = OrderView<'a>;
///     type StaticViewType = OrderView<'static>;
///     fn view(source: &NewOrder) -> OrderView<'_> {
///         OrderView { symbol: &source.symbol, qty: source.qty }
///     }
/// }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Source}` cannot be viewed as `{Self}`",
    note = "implement `View<{Source}>` for your view marker, or use `#[derive(View)]` with `#[source({Source})]`"
)]
pub trait View<Source> {
    /// The view type with the source borrow lifetime.
    type ViewType<'a> where Source: 'a;

    /// The same type with `'static` — for `IntoRefStep` trait resolution.
    /// Must be layout-identical to `ViewType<'a>` for any `'a`.
    type StaticViewType: 'static;

    /// Construct the view from a borrowed source.
    fn view(source: &Source) -> Self::ViewType<'_>;
}

// =============================================================================
// with_view — scoped lifetime erasure
// =============================================================================

/// Constructs a view, erases its lifetime, runs a closure with `&StaticViewType`,
/// then drops the view. The closure cannot leak the reference.
///
/// # Safety argument
///
/// The view borrows from `source` which is alive for this entire call.
/// The transmute erases the borrow lifetime to `'static` so that
/// `RefStepCall<StaticViewType>` (resolved at build time) can accept it.
/// The closure boundary prevents the `'static` reference from escaping —
/// it's dropped before this function returns.
///
/// This is the same pattern as `std::thread::scope` and `crossbeam::scope`.
#[inline(always)]
fn with_view<Source, V, R>(
    source: &Source,
    f: impl FnOnce(&V::StaticViewType) -> R,
) -> R
where
    V: View<Source>,
{
    let view = V::view(source);
    // SAFETY: ViewType<'a> and StaticViewType are the same struct with
    // different lifetime parameters. The transmute is sound because:
    // 1. The layouts are identical (same repr, same fields).
    // 2. The erased 'static lifetime is confined to the closure scope.
    // 3. `source` outlives this entire function, so the borrow is valid.
    // 4. The closure receives `&StaticViewType` (shared ref) — it cannot
    //    store, return, or leak it.
    let static_ref: &V::StaticViewType =
        unsafe { &*(std::ptr::from_ref(&view) as *const V::StaticViewType) };
    let result = f(static_ref);
    drop(view);
    result
}

// =============================================================================
// ViewScope — builder for steps inside a view scope
// =============================================================================

/// Builder for steps inside a `.view::<V>()` scope.
///
/// `V` is the view marker (implements [`View<Out>`]). Steps resolve
/// against `V::StaticViewType` via `IntoRefStep`.
pub struct ViewScope<In, Out, V: View<Out>, PrevChain, InnerSteps> {
    prev_chain: PrevChain,
    inner: InnerSteps,
    _marker: PhantomData<(fn(In) -> Out, V)>,
}

impl<In, Out, V: View<Out>, PrevChain> ViewScope<In, Out, V, PrevChain, ()> {
    pub(crate) fn new(prev_chain: PrevChain) -> Self {
        ViewScope {
            prev_chain,
            inner: (),
            _marker: PhantomData,
        }
    }
}

// --- Combinators ---

impl<In, Out, V: View<Out>, PrevChain, InnerSteps>
    ViewScope<In, Out, V, PrevChain, InnerSteps>
{
    /// Observe the view. Side effects via `Res`/`ResMut`.
    /// Step signature: `fn(Params..., &ViewType) -> ()`.
    pub fn tap<Params, S: IntoRefStep<V::StaticViewType, (), Params>>(
        self,
        f: S,
        registry: &crate::world::Registry,
    ) -> ViewScope<In, Out, V, PrevChain, (InnerSteps, ViewTap<S::Step>)> {
        ViewScope {
            prev_chain: self.prev_chain,
            inner: (self.inner, ViewTap(f.into_ref_step(registry))),
            _marker: PhantomData,
        }
    }

    /// Guard the event based on the view.
    /// Step signature: `fn(Params..., &ViewType) -> bool`.
    pub fn guard<Params, S: IntoRefStep<V::StaticViewType, bool, Params>>(
        self,
        f: S,
        registry: &crate::world::Registry,
    ) -> ViewScope<In, Out, V, PrevChain, (InnerSteps, ViewGuard<S::Step>)> {
        ViewScope {
            prev_chain: self.prev_chain,
            inner: (self.inner, ViewGuard(f.into_ref_step(registry))),
            _marker: PhantomData,
        }
    }
}

// --- Step wrappers ---

#[doc(hidden)]
pub struct ViewTap<S>(S);

#[doc(hidden)]
pub struct ViewGuard<S>(S);

// --- ViewSteps trait ---

#[doc(hidden)]
pub trait ViewSteps<V> {
    fn run(&mut self, world: &mut World, view: &V) -> bool;
}

impl<V> ViewSteps<V> for () {
    fn run(&mut self, _world: &mut World, _view: &V) -> bool { true }
}

impl<V, Prev: ViewSteps<V>, S: RefStepCall<V, Out = ()>> ViewSteps<V> for (Prev, ViewTap<S>) {
    fn run(&mut self, world: &mut World, view: &V) -> bool {
        let ok = self.0.run(world, view);
        self.1 .0.call(world, view);
        ok
    }
}

impl<V, Prev: ViewSteps<V>, S: RefStepCall<V, Out = bool>> ViewSteps<V> for (Prev, ViewGuard<S>) {
    fn run(&mut self, world: &mut World, view: &V) -> bool {
        if !self.0.run(world, view) {
            return false;
        }
        self.1 .0.call(world, view)
    }
}

// =============================================================================
// end_view
// =============================================================================

impl<In, Out, V, PrevChain, InnerSteps> ViewScope<In, Out, V, PrevChain, InnerSteps>
where
    PrevChain: ChainCall<In, Out = Out>,
    V: View<Out>,
    InnerSteps: ViewSteps<V::StaticViewType>,
{
    /// Close the view scope. The event passes through unchanged.
    pub fn end_view(self) -> PipelineChain<In, Out, ViewNode<PrevChain, InnerSteps, V>> {
        PipelineChain {
            chain: ViewNode {
                prev: self.prev_chain,
                inner: self.inner,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }

    /// Close the view scope. If any guard rejected, returns `None`.
    pub fn end_view_guarded(
        self,
    ) -> PipelineChain<In, Option<Out>, ViewGuardedNode<PrevChain, InnerSteps, V>> {
        PipelineChain {
            chain: ViewGuardedNode {
                prev: self.prev_chain,
                inner: self.inner,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Chain nodes
// =============================================================================

#[doc(hidden)]
pub struct ViewNode<Prev, Inner, V> {
    prev: Prev,
    inner: Inner,
    _marker: PhantomData<V>,
}

impl<In, Out, Prev, Inner, V> ChainCall<In> for ViewNode<Prev, Inner, V>
where
    Prev: ChainCall<In, Out = Out>,
    V: View<Out>,
    Inner: ViewSteps<V::StaticViewType>,
{
    type Out = Out;

    fn call(&mut self, world: &mut World, input: In) -> Out {
        let event = self.prev.call(world, input);
        with_view::<Out, V, ()>(&event, |view| {
            self.inner.run(world, view);
        });
        event
    }
}

#[doc(hidden)]
pub struct ViewGuardedNode<Prev, Inner, V> {
    prev: Prev,
    inner: Inner,
    _marker: PhantomData<V>,
}

impl<In, Out, Prev, Inner, V> ChainCall<In> for ViewGuardedNode<Prev, Inner, V>
where
    Prev: ChainCall<In, Out = Out>,
    V: View<Out>,
    Inner: ViewSteps<V::StaticViewType>,
{
    type Out = Option<Out>;

    fn call(&mut self, world: &mut World, input: In) -> Option<Out> {
        let event = self.prev.call(world, input);
        let pass = with_view::<Out, V, bool>(&event, |view| {
            self.inner.run(world, view)
        });
        if pass { Some(event) } else { None }
    }
}

// =============================================================================
// PipelineBuilder / PipelineChain integration
// =============================================================================

impl<In> crate::pipeline::PipelineBuilder<In> {
    /// Open a view scope as the first pipeline step.
    pub fn view<V: View<In>>(self) -> ViewScope<In, In, V, crate::pipeline::IdentityNode, ()> {
        ViewScope::new(crate::pipeline::IdentityNode)
    }
}

impl<In, Out, Chain: ChainCall<In, Out = Out>> PipelineChain<In, Out, Chain> {
    /// Open a view scope. Steps inside operate on a read-only view
    /// constructed from the pipeline's current event.
    ///
    /// `V` is a marker type implementing [`View<Out>`]. Inside the scope,
    /// steps resolve against `V::StaticViewType` — borrowed views work
    /// via lifetime erasure (same pattern as `std::thread::scope`).
    pub fn view<V: View<Out>>(self) -> ViewScope<In, Out, V, Chain, ()> {
        ViewScope::new(self.chain)
    }
}

// =============================================================================
// DagChain / DagArm integration
// =============================================================================

impl<In, Out, V, PrevChain, InnerSteps> ViewScope<In, Out, V, PrevChain, InnerSteps>
where
    PrevChain: ChainCall<In, Out = Out>,
    V: View<Out>,
    InnerSteps: ViewSteps<V::StaticViewType>,
    Out: 'static,
{
    /// Close the view scope, returning a [`DagChain`](crate::dag::DagChain).
    pub fn end_view_dag(self) -> crate::dag::DagChain<In, Out, ViewNode<PrevChain, InnerSteps, V>> {
        crate::dag::DagChain {
            chain: ViewNode {
                prev: self.prev_chain,
                inner: self.inner,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }

    /// Close a guarded view scope, returning a [`DagChain`](crate::dag::DagChain).
    pub fn end_view_dag_guarded(
        self,
    ) -> crate::dag::DagChain<In, Option<Out>, ViewGuardedNode<PrevChain, InnerSteps, V>> {
        crate::dag::DagChain {
            chain: ViewGuardedNode {
                prev: self.prev_chain,
                inner: self.inner,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }

    /// Close the view scope, returning a [`DagArm`](crate::dag::DagArm).
    pub fn end_view_arm(self) -> crate::dag::DagArm<In, Out, ViewNode<PrevChain, InnerSteps, V>> {
        crate::dag::DagArm {
            chain: ViewNode {
                prev: self.prev_chain,
                inner: self.inner,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }

    /// Close a guarded view scope, returning a [`DagArm`](crate::dag::DagArm).
    pub fn end_view_arm_guarded(
        self,
    ) -> crate::dag::DagArm<In, Option<Out>, ViewGuardedNode<PrevChain, InnerSteps, V>> {
        crate::dag::DagArm {
            chain: ViewGuardedNode {
                prev: self.prev_chain,
                inner: self.inner,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PipelineBuilder, Res, ResMut, Resource, WorldBuilder};

    // -- Domain types --

    struct AuditLog(Vec<String>);
    impl Resource for AuditLog {}

    struct RiskLimits { max_qty: u64 }
    impl Resource for RiskLimits {}

    // Borrowed view — the whole point
    struct OrderView<'a> { symbol: &'a str, qty: u64 }

    struct NewOrderCommand {
        source: String,
        symbol: String,
        qty: u64,
        #[allow(dead_code)]
        price: f64,
    }

    struct AmendOrderCommand {
        #[allow(dead_code)]
        order_id: u64,
        symbol: String,
        qty: u64,
        #[allow(dead_code)]
        price: f64,
    }

    // -- View marker + impls (zero-cost borrows) --

    struct AsOrderView;

    impl View<NewOrderCommand> for AsOrderView {
        type ViewType<'a> = OrderView<'a>;
        type StaticViewType = OrderView<'static>;
        fn view(source: &NewOrderCommand) -> OrderView<'_> {
            OrderView { symbol: &source.symbol, qty: source.qty }
        }
    }

    impl View<AmendOrderCommand> for AsOrderView {
        type ViewType<'a> = OrderView<'a>;
        type StaticViewType = OrderView<'static>;
        fn view(source: &AmendOrderCommand) -> OrderView<'_> {
            OrderView { symbol: &source.symbol, qty: source.qty }
        }
    }

    // -- Reusable steps (Params first, &View last) --

    fn log_order(mut log: ResMut<AuditLog>, v: &OrderView) {
        log.0.push(format!("{} qty={}", v.symbol, v.qty));
    }

    fn check_risk(limits: Res<RiskLimits>, v: &OrderView) -> bool {
        v.qty <= limits.max_qty
    }

    // -- Tests --

    #[test]
    fn tap_observes_view() {
        let mut wb = WorldBuilder::new();
        wb.register(AuditLog(Vec::new()));
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineBuilder::<NewOrderCommand>::new()
            .view::<AsOrderView>()
                .tap(log_order, reg)
            .end_view()
            .then(|_cmd: NewOrderCommand| {}, reg);

        p.run(&mut world, NewOrderCommand {
            source: "test".into(), symbol: "BTC".into(), qty: 50, price: 42000.0,
        });

        assert_eq!(world.resource::<AuditLog>().0, vec!["BTC qty=50"]);
    }

    #[test]
    fn guard_rejects() {
        let mut wb = WorldBuilder::new();
        wb.register(AuditLog(Vec::new()));
        wb.register(RiskLimits { max_qty: 100 });
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineBuilder::<NewOrderCommand>::new()
            .view::<AsOrderView>()
                .tap(log_order, reg)
                .guard(check_risk, reg)
            .end_view_guarded();

        let result = p.run(&mut world, NewOrderCommand {
            source: "a".into(), symbol: "BTC".into(), qty: 50, price: 42000.0,
        });
        assert!(result.is_some());

        let result = p.run(&mut world, NewOrderCommand {
            source: "b".into(), symbol: "ETH".into(), qty: 200, price: 3000.0,
        });
        assert!(result.is_none());

        // Tap fires even when guard rejects
        assert_eq!(world.resource::<AuditLog>().0.len(), 2);
    }

    #[test]
    fn event_passes_through_unchanged() {
        let mut wb = WorldBuilder::new();
        wb.register(AuditLog(Vec::new()));
        let mut world = wb.build();
        let reg = world.registry();

        fn sink(mut out: ResMut<AuditLog>, cmd: NewOrderCommand) {
            out.0.push(format!("sink: {} from {}", cmd.symbol, cmd.source));
        }

        let mut p = PipelineBuilder::<NewOrderCommand>::new()
            .view::<AsOrderView>()
                .tap(log_order, reg)
            .end_view()
            .then(sink, reg);

        p.run(&mut world, NewOrderCommand {
            source: "ops".into(), symbol: "SOL".into(), qty: 10, price: 150.0,
        });

        let log = &world.resource::<AuditLog>().0;
        assert_eq!(log[0], "SOL qty=10");
        assert_eq!(log[1], "sink: SOL from ops");
    }

    #[test]
    fn reusable_across_event_types() {
        let mut wb = WorldBuilder::new();
        wb.register(AuditLog(Vec::new()));
        let mut world = wb.build();
        let reg = world.registry();

        let mut p_new = PipelineBuilder::<NewOrderCommand>::new()
            .view::<AsOrderView>()
                .tap(log_order, reg)
            .end_view()
            .then(|_: NewOrderCommand| {}, reg);

        let mut p_amend = PipelineBuilder::<AmendOrderCommand>::new()
            .view::<AsOrderView>()
                .tap(log_order, reg) // SAME function
            .end_view()
            .then(|_: AmendOrderCommand| {}, reg);

        p_new.run(&mut world, NewOrderCommand {
            source: "a".into(), symbol: "BTC".into(), qty: 50, price: 42000.0,
        });
        p_amend.run(&mut world, AmendOrderCommand {
            order_id: 123, symbol: "ETH".into(), qty: 25, price: 3000.0,
        });

        let log = &world.resource::<AuditLog>().0;
        assert_eq!(log[0], "BTC qty=50");
        assert_eq!(log[1], "ETH qty=25");
    }

    #[test]
    fn multiple_taps_in_scope() {
        let mut wb = WorldBuilder::new();
        wb.register(AuditLog(Vec::new()));
        let mut world = wb.build();
        let reg = world.registry();

        fn log_symbol(mut log: ResMut<AuditLog>, v: &OrderView) {
            log.0.push(format!("symbol: {}", v.symbol));
        }
        fn log_qty(mut log: ResMut<AuditLog>, v: &OrderView) {
            log.0.push(format!("qty: {}", v.qty));
        }

        let mut p = PipelineBuilder::<NewOrderCommand>::new()
            .view::<AsOrderView>()
                .tap(log_symbol, reg)
                .tap(log_qty, reg)
            .end_view()
            .then(|_: NewOrderCommand| {}, reg);

        p.run(&mut world, NewOrderCommand {
            source: "a".into(), symbol: "BTC".into(), qty: 50, price: 42000.0,
        });

        let log = &world.resource::<AuditLog>().0;
        assert_eq!(log[0], "symbol: BTC");
        assert_eq!(log[1], "qty: 50");
    }

    #[test]
    fn sequential_views() {
        struct SymbolView<'a> { symbol: &'a str }
        struct QtyView { qty: u64 }

        struct AsSymbolView;
        impl View<NewOrderCommand> for AsSymbolView {
            type ViewType<'a> = SymbolView<'a>;
            type StaticViewType = SymbolView<'static>;
            fn view(source: &NewOrderCommand) -> SymbolView<'_> {
                SymbolView { symbol: &source.symbol }
            }
        }

        struct AsQtyView;
        impl View<NewOrderCommand> for AsQtyView {
            type ViewType<'a> = QtyView;
            type StaticViewType = QtyView;
            fn view(source: &NewOrderCommand) -> QtyView {
                QtyView { qty: source.qty }
            }
        }

        fn log_sym(mut log: ResMut<AuditLog>, v: &SymbolView) {
            log.0.push(format!("sym: {}", v.symbol));
        }
        fn log_qty_view(mut log: ResMut<AuditLog>, v: &QtyView) {
            log.0.push(format!("qty: {}", v.qty));
        }

        let mut wb = WorldBuilder::new();
        wb.register(AuditLog(Vec::new()));
        let mut world = wb.build();
        let reg = world.registry();

        let mut p = PipelineBuilder::<NewOrderCommand>::new()
            .view::<AsSymbolView>()
                .tap(log_sym, reg)
            .end_view()
            .view::<AsQtyView>()
                .tap(log_qty_view, reg)
            .end_view()
            .then(|_: NewOrderCommand| {}, reg);

        p.run(&mut world, NewOrderCommand {
            source: "a".into(), symbol: "BTC".into(), qty: 50, price: 42000.0,
        });

        let log = &world.resource::<AuditLog>().0;
        assert_eq!(log[0], "sym: BTC");
        assert_eq!(log[1], "qty: 50");
    }

    // -- DAG tests --

    #[test]
    fn dag_view_tap() {
        use crate::{DagBuilder, Handler};

        let mut wb = WorldBuilder::new();
        wb.register(AuditLog(Vec::new()));
        let mut world = wb.build();
        let reg = world.registry();

        let dag = DagBuilder::<NewOrderCommand>::new()
            .root(|cmd: NewOrderCommand| cmd, reg)
            .view::<AsOrderView>()
                .tap(log_order, reg)
            .end_view_dag()
            .then(|_cmd: &NewOrderCommand| {}, reg);

        let mut handler = dag.build();
        handler.run(&mut world, NewOrderCommand {
            source: "test".into(), symbol: "BTC".into(), qty: 50, price: 42000.0,
        });

        assert_eq!(world.resource::<AuditLog>().0, vec!["BTC qty=50"]);
    }

    #[test]
    fn dag_view_guard() {
        use crate::{DagBuilder, Handler};

        let mut wb = WorldBuilder::new();
        wb.register(AuditLog(Vec::new()));
        wb.register(RiskLimits { max_qty: 100 });
        let mut world = wb.build();
        let reg = world.registry();

        fn sink(mut log: ResMut<AuditLog>, val: &Option<NewOrderCommand>) {
            if val.is_some() {
                log.0.push("accepted".into());
            } else {
                log.0.push("rejected".into());
            }
        }

        let dag = DagBuilder::<NewOrderCommand>::new()
            .root(|cmd: NewOrderCommand| cmd, reg)
            .view::<AsOrderView>()
                .tap(log_order, reg)
                .guard(check_risk, reg)
            .end_view_dag_guarded()
            .then(sink, reg);

        let mut handler = dag.build();
        handler.run(&mut world, NewOrderCommand {
            source: "a".into(), symbol: "BTC".into(), qty: 50, price: 42000.0,
        });
        handler.run(&mut world, NewOrderCommand {
            source: "b".into(), symbol: "ETH".into(), qty: 200, price: 3000.0,
        });

        let log = &world.resource::<AuditLog>().0;
        assert_eq!(log[0], "BTC qty=50");
        assert_eq!(log[1], "accepted");
        assert_eq!(log[2], "ETH qty=200");
        assert_eq!(log[3], "rejected");
    }
}
