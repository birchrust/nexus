//! DAG scheduler with boolean propagation.
//!
//! The scheduler is installed as a **driver** via [`SchedulerInstaller`].
//! After event handlers process incoming data and write to resources,
//! the scheduler runs reconciliation [`System`](crate::system::System)s
//! in topological order. This two-phase pattern (event → reconcile)
//! separates reactive logic from derived-state computation.
//!
//! Root systems (no upstream dependencies) always run. Non-root
//! systems run only if at least one upstream system returned `true`.
//!
//! # Propagation model
//!
//! Each system returns `bool`. `true` means "my outputs changed, run
//! downstream systems." `false` means "nothing changed, skip downstream."
//! When a system has multiple upstreams, it runs if **any** upstream
//! returned `true` (OR / `any` semantics).
//!
//! Propagation is checked via a `u64` bitmask — each system occupies one
//! bit, and the upstream check is a single AND instruction. This limits
//! the scheduler to [`MAX_SYSTEMS`] (64) systems.
//!
//! # Sequence mechanics
//!
//! The global sequence counter is event-only — the scheduler never
//! calls [`next_sequence`](crate::World::next_sequence). System writes
//! via [`ResMut`](crate::ResMut) stamp at the current (last event's)
//! sequence. At the end of each pass, [`SchedulerTick`] is updated to
//! [`current_sequence`](crate::World::current_sequence).
//!
//! Systems use [`Res::changed_after`](crate::Res::changed_after) with
//! [`SchedulerTick::last`] to detect resources modified since the
//! previous pass.
//!
//! # Invariants
//!
//! - **Topological order**: Systems execute in an order where all
//!   upstreams of a system have already completed. Ties are broken by
//!   insertion order (Kahn's algorithm with FIFO queue).
//! - **No cycles**: The edge graph must be a DAG. Cycles panic at
//!   install time with a diagnostic message.
//! - **Capacity**: At most [`MAX_SYSTEMS`] (64) systems. Exceeding
//!   this panics at install time.
//! - **Deterministic**: Same inputs produce the same execution order
//!   and results. No randomness, no thread-dependent ordering.
//! - **No sequence bump**: The scheduler never advances the global
//!   sequence. Event handlers own sequencing; the scheduler observes.
//!
//! # Examples
//!
//! ```
//! use nexus_rt::{WorldBuilder, Res, ResMut, Installer};
//! use nexus_rt::scheduler::SchedulerInstaller;
//! use nexus_rt::system::IntoSystem;
//!
//! fn source(mut val: ResMut<u64>) -> bool {
//!     *val += 1;
//!     true
//! }
//!
//! fn sink(val: Res<u64>) -> bool {
//!     *val > 0
//! }
//!
//! let mut builder = WorldBuilder::new();
//! builder.register::<u64>(0);
//!
//! let mut installer = SchedulerInstaller::new();
//! let a = installer.add(source, builder.registry());
//! let b = installer.add(sink, builder.registry());
//! installer.after(b, a);
//!
//! let mut scheduler = builder.install_driver(installer);
//! let mut world = builder.build();
//!
//! assert_eq!(scheduler.run(&mut world), 2);
//! ```

use std::collections::VecDeque;

use crate::driver::Installer;
use crate::system::{IntoSystem, System};
use crate::world::{Registry, ResourceId, Sequence, World, WorldBuilder};

// =============================================================================
// SchedulerTick
// =============================================================================

/// Tracks the sequence at which the last scheduler pass completed.
///
/// Registered as a resource by [`SchedulerInstaller::install`]. Systems
/// can use [`Res::changed_after`](crate::Res::changed_after) with
/// `scheduler_tick.last()` to detect changes since the previous pass.
///
/// Updated at the end of each [`SystemScheduler::run`] call to
/// [`World::current_sequence`](crate::World::current_sequence).
///
/// # Examples
///
/// ```
/// use nexus_rt::{Res, ResMut};
/// use nexus_rt::scheduler::SchedulerTick;
///
/// fn detect_and_reconcile(
///     val: Res<u64>,
///     tick: Res<SchedulerTick>,
///     mut out: ResMut<bool>,
/// ) -> bool {
///     if val.changed_after(tick.last()) {
///         *out = *val > 100;
///         true
///     } else {
///         false
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct SchedulerTick(Sequence);

impl SchedulerTick {
    /// The sequence at which the last scheduler pass completed.
    pub fn last(&self) -> Sequence {
        self.0
    }
}

// =============================================================================
// SystemId
// =============================================================================

/// Opaque handle identifying a system within a [`SchedulerInstaller`].
///
/// Used with [`after`](SchedulerInstaller::after) and
/// [`before`](SchedulerInstaller::before) to declare ordering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SystemId(usize);

// =============================================================================
// SchedulerInstaller
// =============================================================================

/// Builder for a [`SystemScheduler`].
///
/// Systems are added with [`add`](Self::add) and ordering declared with
/// [`after`](Self::after) / [`before`](Self::before). On
/// [`install`](Installer::install), a topological sort (Kahn's algorithm
/// with FIFO queue for deterministic insertion-order tie-breaking)
/// determines execution order.
///
/// Implements [`Installer`] — consume via
/// [`WorldBuilder::install_driver`](crate::WorldBuilder::install_driver).
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, Res, ResMut, Installer};
/// use nexus_rt::scheduler::SchedulerInstaller;
/// use nexus_rt::system::IntoSystem;
///
/// fn step_a(mut val: ResMut<u64>) -> bool {
///     *val += 1;
///     true
/// }
///
/// fn step_b(val: Res<u64>) -> bool {
///     *val > 0
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
///
/// let mut installer = SchedulerInstaller::new();
/// let a = installer.add(step_a, builder.registry());
/// let b = installer.add(step_b, builder.registry());
/// installer.after(b, a);
///
/// let mut scheduler = builder.install_driver(installer);
/// let mut world = builder.build();
///
/// assert_eq!(scheduler.run(&mut world), 2);
/// ```
///
/// # Panics
///
/// [`install`](Installer::install) panics if:
/// - The declared edges form a cycle.
/// - More than [`MAX_SYSTEMS`] (64) systems were added.
pub struct SchedulerInstaller {
    systems: Vec<Box<dyn System>>,
    edges: Vec<(usize, usize)>, // (upstream, downstream)
}

impl SchedulerInstaller {
    /// Create an empty installer.
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Add a system, returning a [`SystemId`] for ordering.
    ///
    /// The function is converted via [`IntoSystem`] and parameters
    /// are resolved from the registry immediately.
    ///
    /// # Panics
    ///
    /// Panics if any [`Param`](crate::Param) resource required by `f`
    /// is not registered in the [`Registry`], or if parameters create
    /// a conflicting access (e.g. `Res<T>` + `ResMut<T>`).
    pub fn add<F, Params>(&mut self, f: F, registry: &Registry) -> SystemId
    where
        F: IntoSystem<Params>,
        F::System: 'static,
    {
        let id = SystemId(self.systems.len());
        self.systems.push(Box::new(f.into_system(registry)));
        id
    }

    /// Declare that `downstream` runs after `upstream`.
    ///
    /// If `upstream` returns `false`, `downstream` is skipped — unless
    /// another upstream of `downstream` returned `true` (`any` semantics).
    pub fn after(&mut self, downstream: SystemId, upstream: SystemId) {
        self.edges.push((upstream.0, downstream.0));
    }

    /// Declare that `upstream` runs before `downstream`.
    ///
    /// Equivalent to `self.after(downstream, upstream)`.
    pub fn before(&mut self, upstream: SystemId, downstream: SystemId) {
        self.after(downstream, upstream);
    }
}

impl Default for SchedulerInstaller {
    fn default() -> Self {
        Self::new()
    }
}

/// Maximum number of systems supported by the scheduler.
///
/// Propagation uses a `u64` bitmask where each bit represents one
/// system's result. The upstream check for any system is a single AND
/// against this bitmask — no iteration over dependency lists.
///
/// 64 systems is well beyond typical reconciliation DAG sizes. If a
/// future use case requires more, the bitmask can be widened to `u128`
/// or `[u64; N]` without changing the public API.
pub const MAX_SYSTEMS: usize = 64;

impl Installer for SchedulerInstaller {
    type Poller = SystemScheduler;

    fn install(self, world: &mut WorldBuilder) -> SystemScheduler {
        let tick_id = world.ensure(SchedulerTick::default());
        let n = self.systems.len();

        assert!(
            n <= MAX_SYSTEMS,
            "system scheduler supports at most {MAX_SYSTEMS} systems, got {n}",
        );

        // Build adjacency list and in-degree counts.
        let mut in_degree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

        for &(up, down) in &self.edges {
            adj[up].push(down);
            in_degree[down] += 1;
        }

        // Kahn's algorithm — FIFO queue for stable insertion-order.
        let mut queue = VecDeque::new();
        for (i, deg) in in_degree.iter().enumerate() {
            if *deg == 0 {
                queue.push_back(i);
            }
        }

        let mut order: Vec<usize> = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            order.push(node);
            for &succ in &adj[node] {
                in_degree[succ] -= 1;
                if in_degree[succ] == 0 {
                    queue.push_back(succ);
                }
            }
        }

        assert!(
            order.len() == n,
            "cycle detected in system scheduler: {} systems in graph, \
             but only {} reachable in topological order",
            n,
            order.len(),
        );

        // Build the old→new position mapping.
        let mut old_to_new = vec![0usize; n];
        for (new_pos, &old_pos) in order.iter().enumerate() {
            old_to_new[old_pos] = new_pos;
        }

        // Reorder systems into topological order.
        let mut sorted_systems: Vec<Option<Box<dyn System>>> =
            self.systems.into_iter().map(Some).collect();
        let systems: Vec<Box<dyn System>> = order
            .iter()
            .map(|&old_pos| sorted_systems[old_pos].take().unwrap())
            .collect();

        // Build upstream bitmasks: for each system in topological order,
        // a u64 where bit j is set if system j is an upstream dependency.
        // Roots have mask 0 (always run).
        let mut upstream_masks = vec![0u64; n];
        for &(up, down) in &self.edges {
            upstream_masks[old_to_new[down]] |= 1 << old_to_new[up];
        }

        SystemScheduler {
            tick_id,
            systems,
            upstream_masks,
        }
    }
}

// =============================================================================
// SystemScheduler
// =============================================================================

/// DAG scheduler that runs systems in topological order with boolean
/// propagation.
///
/// Created by [`SchedulerInstaller`] via
/// [`WorldBuilder::install_driver`](crate::WorldBuilder::install_driver).
/// This is a user-space driver — the caller decides when and whether
/// to call [`run`](Self::run).
///
/// # Propagation
///
/// Root systems (no upstream) always run. Non-root systems run only if
/// at least one upstream returned `true`.
///
/// # Bitmask implementation
///
/// Each system occupies one bit position in a `u64`. During
/// [`run`](Self::run), a local `results: u64` accumulates which systems
/// returned `true`. Each system's upstream check is:
///
/// ```text
/// mask == 0              → root, always run
/// (mask & results) != 0  → at least one upstream returned true
/// ```
///
/// One load, one AND, one branch per system — no heap access for the
/// propagation check itself. The `results` bitmask lives in a register
/// for the duration of the loop.
pub struct SystemScheduler {
    tick_id: ResourceId,
    systems: Vec<Box<dyn System>>,
    /// Per-system: bit `j` set means system `j` is an upstream dependency.
    /// `0` = root (always runs).
    upstream_masks: Vec<u64>,
}

impl SystemScheduler {
    /// Run all systems with boolean propagation.
    ///
    /// Iterates systems in topological order. For each system:
    ///
    /// 1. **Root** (`upstream_mask == 0`): always runs.
    /// 2. **Non-root** (`upstream_mask & results != 0`): runs if any
    ///    upstream returned `true`. Skipped otherwise.
    /// 3. If the system runs and returns `true`, its bit is set in
    ///    `results`, enabling downstream systems to run.
    ///
    /// The `results` bitmask is a local `u64` — the entire propagation
    /// state fits in a single register. Per-system overhead is one
    /// load + one AND + one branch (~4 cycles).
    ///
    /// Does NOT call [`next_sequence`](World::next_sequence) — the global
    /// sequence is event-only. System writes via [`ResMut`](crate::ResMut)
    /// stamp at the current (last event's) sequence.
    ///
    /// Updates [`SchedulerTick`] to
    /// [`current_sequence`](World::current_sequence) at the end of
    /// each pass.
    ///
    /// Returns the number of systems that actually ran.
    pub fn run(&mut self, world: &mut World) -> usize {
        let mut ran = 0;
        let mut results: u64 = 0;

        for i in 0..self.systems.len() {
            let mask = self.upstream_masks[i];
            if mask == 0 || (mask & results) != 0 {
                if self.systems[i].run(world) {
                    results |= 1 << i;
                }
                ran += 1;
            }
        }

        // SAFETY: tick_id was obtained from ensure() on the same builder.
        unsafe {
            world.get_mut::<SchedulerTick>(self.tick_id).0 = world.current_sequence();
        }

        ran
    }

    /// Returns the number of systems in the scheduler.
    pub fn len(&self) -> usize {
        self.systems.len()
    }

    /// Returns `true` if the scheduler contains no systems.
    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Res, ResMut};

    // -- Empty scheduler --------------------------------------------------

    #[test]
    fn empty_scheduler() {
        let mut builder = WorldBuilder::new();
        let installer = SchedulerInstaller::new();
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        assert_eq!(scheduler.run(&mut world), 0);
        assert!(scheduler.is_empty());
    }

    // -- Single root ------------------------------------------------------

    fn increment(mut val: ResMut<u64>) -> bool {
        *val += 1;
        true
    }

    #[test]
    fn single_root_always_runs() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut installer = SchedulerInstaller::new();
        installer.add(increment, builder.registry());
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        assert_eq!(scheduler.run(&mut world), 1);
        assert_eq!(*world.resource::<u64>(), 1);
    }

    // -- Linear chain: A → B → C -----------------------------------------

    fn source(mut val: ResMut<u64>) -> bool {
        *val += 1;
        *val <= 2 // propagate first two times
    }

    fn middle(mut val: ResMut<u64>) -> bool {
        *val += 10;
        true
    }

    fn leaf(mut val: ResMut<u64>) -> bool {
        *val += 100;
        true
    }

    #[test]
    fn linear_chain_propagation() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut installer = SchedulerInstaller::new();
        let a = installer.add(source, builder.registry());
        let b = installer.add(middle, builder.registry());
        let c = installer.add(leaf, builder.registry());
        installer.after(b, a);
        installer.after(c, b);
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        // Pass 1: source returns true → all 3 run
        assert_eq!(scheduler.run(&mut world), 3);
        // 0 + 1 + 10 + 100 = 111
        assert_eq!(*world.resource::<u64>(), 111);
    }

    // -- Propagation stops ------------------------------------------------

    fn false_source() -> bool {
        false
    }

    fn should_not_run(mut val: ResMut<u64>) -> bool {
        *val = 999;
        true
    }

    #[test]
    fn propagation_stops_on_false() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut installer = SchedulerInstaller::new();
        let a = installer.add(false_source, builder.registry());
        let b = installer.add(should_not_run, builder.registry());
        installer.after(b, a);
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        // Only root runs, downstream skipped
        assert_eq!(scheduler.run(&mut world), 1);
        assert_eq!(*world.resource::<u64>(), 0);
    }

    // -- Diamond DAG: A → B, A → C, B → D, C → D -------------------------

    fn set_flag(mut flag: ResMut<bool>) -> bool {
        *flag = true;
        true
    }

    #[test]
    fn diamond_dag() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        builder.register::<bool>(false);
        let mut installer = SchedulerInstaller::new();
        let a = installer.add(increment, builder.registry());
        let b = installer.add(increment, builder.registry());
        let c = installer.add(set_flag, builder.registry());
        let d = installer.add(increment, builder.registry());
        installer.after(b, a);
        installer.after(c, a);
        installer.after(d, b);
        installer.after(d, c);
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        assert_eq!(scheduler.run(&mut world), 4);
        assert!(*world.resource::<bool>());
        // u64: increment runs 3 times (a, b, d) → 3
        assert_eq!(*world.resource::<u64>(), 3);
    }

    // -- Multiple roots ---------------------------------------------------

    #[test]
    fn multiple_roots() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut installer = SchedulerInstaller::new();
        installer.add(increment, builder.registry());
        installer.add(increment, builder.registry());
        installer.add(increment, builder.registry());
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        assert_eq!(scheduler.run(&mut world), 3);
        assert_eq!(*world.resource::<u64>(), 3);
    }

    // -- Cycle detection --------------------------------------------------

    #[test]
    #[should_panic(expected = "cycle detected")]
    fn cycle_panics() {
        let mut builder = WorldBuilder::new();
        let mut installer = SchedulerInstaller::new();
        let a = installer.add(false_source, builder.registry());
        let b = installer.add(false_source, builder.registry());
        installer.after(b, a);
        installer.after(a, b);
        let _scheduler = builder.install_driver(installer);
    }

    // -- SchedulerTick updated --------------------------------------------

    #[test]
    fn scheduler_tick_updated() {
        let mut builder = WorldBuilder::new();
        let mut installer = SchedulerInstaller::new();
        installer.add(|| -> bool { true }, builder.registry());
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        // Advance sequence as if events were processed.
        world.next_sequence(); // 1
        world.next_sequence(); // 2

        scheduler.run(&mut world);

        assert_eq!(world.resource::<SchedulerTick>().last(), Sequence(2));
    }

    // -- changed_after integration ----------------------------------------

    fn detect_change(val: Res<u64>, tick: Res<SchedulerTick>) -> bool {
        val.changed_after(tick.last())
    }

    #[test]
    fn changed_after_integration() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        // Pre-register SchedulerTick so detect_change can resolve it.
        // install() uses ensure(), so the duplicate is harmless.
        builder.ensure(SchedulerTick::default());
        let mut installer = SchedulerInstaller::new();
        installer.add(detect_change, builder.registry());
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        // First pass: changed_at=0, tick.last()=0 → 0 > 0 is false
        scheduler.run(&mut world);

        // Simulate event that writes u64
        world.next_sequence(); // seq=1
        *world.resource_mut::<u64>() = 42; // stamps changed_at=1

        // Second pass: changed_at=1, tick.last()=0 → 1 > 0 is true
        // (tick updated at END of pass — during this pass it's still 0)
        scheduler.run(&mut world);
        // After this pass, tick.last() = 1.

        // Third pass: no new events, changed_at=1, tick.last()=1 → 1 > 1 = false
        scheduler.run(&mut world);
    }

    // -- Sequence not bumped by scheduler ---------------------------------

    #[test]
    fn scheduler_does_not_bump_sequence() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut installer = SchedulerInstaller::new();
        installer.add(increment, builder.registry());
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        let before = world.current_sequence();
        scheduler.run(&mut world);
        assert_eq!(world.current_sequence(), before);
    }

    // -- System mutations visible to later systems ------------------------

    fn double(mut val: ResMut<u64>) -> bool {
        *val *= 2;
        true
    }

    #[test]
    fn mutations_visible_downstream() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(1);
        let mut installer = SchedulerInstaller::new();
        let a = installer.add(double, builder.registry());
        let b = installer.add(double, builder.registry());
        installer.after(b, a);
        let mut scheduler = builder.install_driver(installer);
        let mut world = builder.build();

        scheduler.run(&mut world);
        // 1 * 2 = 2, then 2 * 2 = 4
        assert_eq!(*world.resource::<u64>(), 4);
    }

    // -- Capacity limit ---------------------------------------------------

    #[test]
    #[should_panic(expected = "at most 64 systems")]
    fn exceeding_max_systems_panics() {
        let mut builder = WorldBuilder::new();
        let mut installer = SchedulerInstaller::new();
        for _ in 0..=MAX_SYSTEMS {
            installer.add(false_source, builder.registry());
        }
        let _scheduler = builder.install_driver(installer);
    }
}
