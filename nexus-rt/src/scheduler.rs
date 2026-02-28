//! Toposorted system dispatch with automatic skip propagation.
//!
//! [`SchedulerBuilder`] collects systems and ordering constraints at build
//! time. [`SchedulerBuilder::build`] toposorts into a flat [`Scheduler`]
//! that walks the schedule, skipping systems whose inputs haven't changed.
//!
//! # Skip Propagation
//!
//! Before running each system, the Scheduler checks
//! [`System::inputs_changed`]. If nothing changed, the system is skipped.
//! Skipped systems don't write, so downstream systems also see unchanged
//! inputs — propagation is natural, no bookkeeping.
//!
//! # Usage
//!
//! ```ignore
//! world.with_mut::<Scheduler, _>(|scheduler, world| {
//!     scheduler.dispatch(world);
//! });
//! ```

use std::collections::VecDeque;

use crate::World;
use crate::system::{IntoSystem, System};
use crate::world::Registry;

/// Boxed run condition predicate. Evaluated before dispatch — if it
/// returns `false`, the system is skipped entirely.
type Condition = Box<dyn Fn(&World) -> bool>;

// =============================================================================
// SystemId
// =============================================================================

/// Opaque handle identifying a system within a [`SchedulerBuilder`].
///
/// Returned by [`SchedulerBuilder::add_system`], used with
/// [`SchedulerBuilder::after`] to declare ordering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SystemId(usize);

// =============================================================================
// SchedulerBuilder
// =============================================================================

struct BuildEntry {
    system: Box<dyn System<()>>,
    after: Vec<SystemId>,
    condition: Option<Condition>,
}

/// Builder for constructing a toposorted [`Scheduler`].
///
/// Systems are added with [`add_system`](Self::add_system) and ordering
/// constraints declared with [`after`](Self::after). Call
/// [`build`](Self::build) to produce the final [`Scheduler`].
pub struct SchedulerBuilder {
    entries: Vec<BuildEntry>,
}

impl SchedulerBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Register a system, returning its [`SystemId`] handle.
    ///
    /// The system's parameters are resolved against the provided registry.
    /// Panics if any required resource is not registered.
    pub fn add_system<P>(
        &mut self,
        system: impl IntoSystem<(), P>,
        registry: &Registry,
    ) -> SystemId {
        let id = SystemId(self.entries.len());
        self.entries.push(BuildEntry {
            system: Box::new(system.into_system(registry)),
            after: Vec::new(),
            condition: None,
        });
        id
    }

    /// Declare that `system` must run after `dependency`.
    ///
    /// # Panics
    ///
    /// Panics if either [`SystemId`] is out of bounds.
    pub fn after(&mut self, system: SystemId, dependency: SystemId) -> &mut Self {
        assert!(
            system.0 < self.entries.len(),
            "SystemId({}) out of bounds (len {})",
            system.0,
            self.entries.len(),
        );
        assert!(
            dependency.0 < self.entries.len(),
            "SystemId({}) out of bounds (len {})",
            dependency.0,
            self.entries.len(),
        );
        self.entries[system.0].after.push(dependency);
        self
    }

    /// Attach a run condition to a system.
    ///
    /// The condition is evaluated before [`System::inputs_changed`].
    /// If it returns `false`, the system is skipped entirely — it never
    /// runs, never writes, and downstream systems see unchanged inputs.
    ///
    /// Only one condition per system. A second call replaces the first.
    ///
    /// # Panics
    ///
    /// Panics if the [`SystemId`] is out of bounds.
    pub fn run_if(
        &mut self,
        system: SystemId,
        condition: impl Fn(&World) -> bool + 'static,
    ) -> &mut Self {
        assert!(
            system.0 < self.entries.len(),
            "SystemId({}) out of bounds (len {})",
            system.0,
            self.entries.len(),
        );
        self.entries[system.0].condition = Some(Box::new(condition));
        self
    }

    /// Toposort and produce the final [`Scheduler`].
    ///
    /// # Panics
    ///
    /// Panics if the ordering constraints contain a cycle.
    pub fn build(self) -> Scheduler {
        let n = self.entries.len();
        if n == 0 {
            return Scheduler {
                schedule: Vec::new(),
            };
        }

        // Build adjacency list: adj[dep] contains systems that depend on dep.
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_degree: Vec<usize> = vec![0; n];

        for (idx, entry) in self.entries.iter().enumerate() {
            for dep in &entry.after {
                adj[dep.0].push(idx);
                in_degree[idx] += 1;
            }
        }

        // Kahn's algorithm with FIFO queue to preserve registration order
        // among unconstrained peers.
        let mut queue = VecDeque::with_capacity(n);
        for (i, &deg) in in_degree.iter().enumerate() {
            if deg == 0 {
                queue.push_back(i);
            }
        }

        let mut sorted: Vec<usize> = Vec::with_capacity(n);
        while let Some(idx) = queue.pop_front() {
            sorted.push(idx);
            for &dependent in &adj[idx] {
                in_degree[dependent] -= 1;
                if in_degree[dependent] == 0 {
                    queue.push_back(dependent);
                }
            }
        }

        assert_eq!(
            sorted.len(),
            n,
            "cycle detected in system ordering constraints ({} systems in cycle)",
            n - sorted.len(),
        );

        // Reorder entries into toposorted order.
        let mut entries: Vec<Option<BuildEntry>> = self.entries.into_iter().map(Some).collect();
        let schedule = sorted
            .into_iter()
            .map(|idx| {
                let entry = entries[idx].take().expect("index visited twice");
                ScheduleSlot {
                    system: entry.system,
                    condition: entry.condition,
                }
            })
            .collect();

        Scheduler { schedule }
    }
}

impl Default for SchedulerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Scheduler
// =============================================================================

struct ScheduleSlot {
    system: Box<dyn System<()>>,
    condition: Option<Condition>,
}

/// Toposorted system schedule with automatic skip propagation.
///
/// Created by [`SchedulerBuilder::build`]. Stored in [`World`] as a
/// resource, extracted via [`World::with_mut`] for dispatch.
pub struct Scheduler {
    schedule: Vec<ScheduleSlot>,
}

impl Scheduler {
    /// Run the schedule against the world.
    ///
    /// For each system in toposorted order:
    /// 1. If a run condition is attached and returns `false`, skip.
    /// 2. If [`System::inputs_changed`] returns `false`, skip.
    /// 3. Otherwise, run the system.
    ///
    /// Skipped systems don't write, so downstream systems also see
    /// unchanged inputs — skip propagation is automatic regardless
    /// of skip reason.
    pub fn dispatch(&mut self, world: &mut World) {
        for slot in &mut self.schedule {
            let condition_ok = slot.condition.as_ref().is_none_or(|cond| cond(world));
            if condition_ok && slot.system.inputs_changed(world) {
                slot.system.run(world, ());
            }
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Res, ResMut, WorldBuilder};

    type ExecLog = Vec<u64>;

    fn system_a(mut log: ResMut<ExecLog>, _: ()) {
        log.push(0);
    }

    fn system_b(mut log: ResMut<ExecLog>, _: ()) {
        log.push(1);
    }

    fn system_c(mut log: ResMut<ExecLog>, _: ()) {
        log.push(2);
    }

    fn system_d(mut log: ResMut<ExecLog>, _: ()) {
        log.push(3);
    }

    #[test]
    fn single_system_dispatches() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        sb.add_system(system_a, wb.registry());

        wb.register(sb.build());
        let mut world = wb.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert_eq!(*world.resource::<ExecLog>(), vec![0]);
    }

    #[test]
    fn ordering_respected() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        // Register B first, then A, but declare B runs after A.
        let b = sb.add_system(system_b, wb.registry());
        let a = sb.add_system(system_a, wb.registry());
        sb.after(b, a);

        wb.register(sb.build());
        let mut world = wb.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        // A (push 0) runs before B (push 1).
        assert_eq!(*world.resource::<ExecLog>(), vec![0, 1]);
    }

    #[test]
    fn diamond_ordering() {
        // A → B, A → C, B → D, C → D
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        let a = sb.add_system(system_a, wb.registry());
        let b = sb.add_system(system_b, wb.registry());
        let c = sb.add_system(system_c, wb.registry());
        let d = sb.add_system(system_d, wb.registry());

        sb.after(b, a);
        sb.after(c, a);
        sb.after(d, b);
        sb.after(d, c);

        wb.register(sb.build());
        let mut world = wb.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));

        let log = world.resource::<ExecLog>();
        assert_eq!(log[0], 0, "A must run first");
        assert_eq!(log[3], 3, "D must run last");
        assert!(log[1..3].contains(&1), "B must run before D");
        assert!(log[1..3].contains(&2), "C must run before D");
    }

    #[test]
    #[should_panic(expected = "cycle detected")]
    fn cycle_panics() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        let a = sb.add_system(system_a, wb.registry());
        let b = sb.add_system(system_b, wb.registry());
        sb.after(a, b);
        sb.after(b, a);

        sb.build();
    }

    #[test]
    #[should_panic(expected = "cycle detected")]
    fn self_dependency_panics() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        let a = sb.add_system(system_a, wb.registry());
        sb.after(a, a);

        sb.build();
    }

    #[test]
    fn skip_propagation() {
        // Chain: producer reads trigger, writes intermediate.
        //        consumer reads intermediate, writes result.
        // When trigger is stale → producer skipped → intermediate stale →
        // consumer skipped.
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<String>(String::new());
        wb.register::<bool>(false);

        fn producer(trigger: Res<u64>, mut out: ResMut<String>, _: ()) {
            out.push_str(&trigger.to_string());
        }

        fn consumer(input: Res<String>, mut result: ResMut<bool>, _: ()) {
            *result = !input.is_empty();
        }

        let mut sb = SchedulerBuilder::new();
        let p = sb.add_system(producer, wb.registry());
        let c = sb.add_system(consumer, wb.registry());
        sb.after(c, p);

        wb.register(sb.build());
        let mut world = wb.build();

        // Tick 0: all changed_at=0 == current_tick=0 → all run.
        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert!(
            *world.resource::<bool>(),
            "consumer should have run on tick 0"
        );

        // Reset result. resource_mut stamps bool at tick 0.
        *world.resource_mut::<bool>() = false;

        // Advance past all stamps.
        world.advance_tick(); // tick=1
        world.advance_tick(); // tick=2

        // Tick 2: trigger(0) ≠ 2, String(0) ≠ 2, bool(0) ≠ 2 → all stale.
        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert!(
            !*world.resource::<bool>(),
            "consumer should have been skipped"
        );
    }

    #[test]
    fn skip_propagation_fan_out() {
        // Fan-out: source reads trigger, writes to out_a and out_b.
        //          sink_a reads out_a, sink_b reads out_b.
        // When trigger is stale → source skipped → both sinks skipped.
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<String>(String::new());
        wb.register::<f64>(0.0);
        wb.register::<bool>(false);
        wb.register::<i32>(0);

        fn source(trigger: Res<u64>, mut out_a: ResMut<String>, mut out_b: ResMut<f64>, _: ()) {
            out_a.push_str(&trigger.to_string());
            *out_b = *trigger as f64;
        }

        fn sink_a(input: Res<String>, mut flag: ResMut<bool>, _: ()) {
            *flag = !input.is_empty();
        }

        fn sink_b(input: Res<f64>, mut count: ResMut<i32>, _: ()) {
            if *input > 0.0 {
                *count += 1;
            }
        }

        let mut sb = SchedulerBuilder::new();
        let s = sb.add_system(source, wb.registry());
        let a = sb.add_system(sink_a, wb.registry());
        let b = sb.add_system(sink_b, wb.registry());
        sb.after(a, s);
        sb.after(b, s);

        wb.register(sb.build());
        let mut world = wb.build();

        // Tick 0: all changed → all run.
        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert!(*world.resource::<bool>());

        // Advance past all stamps.
        world.advance_tick();
        world.advance_tick();

        // Tick 2: trigger stale → source skipped → out_a/out_b stale →
        // sink_a and sink_b also skipped.
        let count_before = *world.resource::<i32>();
        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert_eq!(
            *world.resource::<i32>(),
            count_before,
            "sink_b should have been skipped"
        );
    }

    // -- Run condition tests ----------------------------------------------------

    #[test]
    fn run_if_false_skips_system() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        let a = sb.add_system(system_a, wb.registry());
        sb.run_if(a, |_| false);

        wb.register(sb.build());
        let mut world = wb.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert!(world.resource::<ExecLog>().is_empty());
    }

    #[test]
    fn run_if_true_allows_system() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        let a = sb.add_system(system_a, wb.registry());
        sb.run_if(a, |_| true);

        wb.register(sb.build());
        let mut world = wb.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert_eq!(*world.resource::<ExecLog>(), vec![0]);
    }

    #[test]
    fn run_if_skips_even_when_inputs_changed() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        let a = sb.add_system(system_a, wb.registry());
        sb.run_if(a, |_| false);

        wb.register(sb.build());
        let mut world = wb.build();

        // Tick 0: ExecLog changed_at=0 == current_tick=0 → inputs changed.
        // But condition is false → skipped anyway.
        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert!(world.resource::<ExecLog>().is_empty());
    }

    #[test]
    fn run_if_reads_resource() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();
        wb.register::<bool>(false); // feature flag

        let mut sb = SchedulerBuilder::new();
        let a = sb.add_system(system_a, wb.registry());
        sb.run_if(a, |world| *world.resource::<bool>());

        wb.register(sb.build());
        let mut world = wb.build();

        // Flag is false → system skipped.
        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert!(world.resource::<ExecLog>().is_empty());

        // Set flag to true.
        *world.resource_mut::<bool>() = true;

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert_eq!(*world.resource::<ExecLog>(), vec![0]);
    }

    #[test]
    fn run_if_skip_propagates_downstream() {
        // A (gated) → B. Condition false → A skipped → B inputs stale → B skipped.
        let mut wb = WorldBuilder::new();
        wb.register::<u64>(0);
        wb.register::<bool>(false);

        fn writer(mut val: ResMut<u64>, _: ()) {
            *val += 1;
        }

        fn reader(val: Res<u64>, mut flag: ResMut<bool>, _: ()) {
            *flag = *val > 0;
        }

        let mut sb = SchedulerBuilder::new();
        let w = sb.add_system(writer, wb.registry());
        let r = sb.add_system(reader, wb.registry());
        sb.after(r, w);
        sb.run_if(w, |_| false);

        wb.register(sb.build());
        let mut world = wb.build();

        // Tick 0: writer gated off → u64 not written.
        // reader: inputs are u64 (changed_at=0==0, still "changed" on tick 0).
        // So reader WILL run on tick 0 because everything starts as "changed."
        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        // u64 is still 0, so reader ran but set flag = false (0 > 0 is false).
        assert!(!*world.resource::<bool>());
        assert_eq!(*world.resource::<u64>(), 0, "writer should not have run");

        // Advance past tick 0 stamps.
        world.advance_tick();
        world.advance_tick();

        // Tick 2: writer still gated → u64 not stamped → reader inputs stale → skipped.
        *world.resource_mut::<bool>() = true; // set to true so we can detect skip
        world.advance_tick(); // tick 3

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert!(
            *world.resource::<bool>(),
            "reader should have been skipped (flag unchanged)"
        );
    }

    #[test]
    fn unconstrained_run_in_registration_order() {
        let mut wb = WorldBuilder::new();
        wb.register_default::<ExecLog>();

        let mut sb = SchedulerBuilder::new();
        sb.add_system(system_a, wb.registry());
        sb.add_system(system_b, wb.registry());
        sb.add_system(system_c, wb.registry());

        wb.register(sb.build());
        let mut world = wb.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert_eq!(*world.resource::<ExecLog>(), vec![0, 1, 2]);
    }
}
