//! Convenience builder for assembling a runtime from plugins.

use crate::plugin::Plugin;
use crate::scheduler::SchedulerBuilder;
use crate::world::{World, WorldBuilder};

/// Convenience builder that ties [`WorldBuilder`] and [`SchedulerBuilder`]
/// together. Registers the built [`Scheduler`] into World automatically.
///
/// # Examples
///
/// ```ignore
/// let mut app = App::new();
/// app.add_plugin(TradingPlugin);
/// let mut world = app.build();
///
/// world.with_mut::<Scheduler, _>(|scheduler, world| {
///     scheduler.dispatch(world);
/// });
/// ```
pub struct App {
    world: WorldBuilder,
    scheduler: SchedulerBuilder,
}

impl App {
    /// Create an empty App.
    pub fn new() -> Self {
        Self {
            world: WorldBuilder::new(),
            scheduler: SchedulerBuilder::new(),
        }
    }

    /// Apply a plugin. Calls [`Plugin::build`] with access to both
    /// the [`WorldBuilder`] and [`SchedulerBuilder`].
    pub fn add_plugin(&mut self, plugin: &impl Plugin) -> &mut Self {
        plugin.build(&mut self.world, &mut self.scheduler);
        self
    }

    /// Access the [`WorldBuilder`] directly.
    pub fn world_mut(&mut self) -> &mut WorldBuilder {
        &mut self.world
    }

    /// Access the [`SchedulerBuilder`] directly.
    pub fn scheduler_mut(&mut self) -> &mut SchedulerBuilder {
        &mut self.scheduler
    }

    /// Build the scheduler, register it into World, and freeze.
    ///
    /// The [`Scheduler`] is available as a resource in the returned
    /// [`World`]. Extract it via [`World::with_mut`] for dispatch.
    pub fn build(self) -> World {
        let scheduler = self.scheduler.build();
        let mut world = self.world;
        world.register(scheduler);
        world.build()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResMut, Scheduler, WorldBuilder};

    type ExecLog = Vec<u64>;

    fn sys_a(mut log: ResMut<ExecLog>, _: ()) {
        log.push(0);
    }

    fn sys_b(mut log: ResMut<ExecLog>, _: ()) {
        log.push(1);
    }

    struct TestPlugin;

    impl Plugin for TestPlugin {
        fn build(&self, world: &mut WorldBuilder, scheduler: &mut SchedulerBuilder) {
            world.register_default::<ExecLog>();
            scheduler.add_system(sys_a, world.registry_mut());
        }
    }

    #[test]
    fn plugin_registers_resources_and_systems() {
        let mut app = App::new();
        app.add_plugin(&TestPlugin);
        let mut world = app.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert_eq!(*world.resource::<ExecLog>(), vec![0]);
    }

    #[test]
    fn app_build_produces_working_world() {
        let mut app = App::new();
        app.add_plugin(&TestPlugin);
        let world = app.build();

        assert!(world.contains::<ExecLog>());
        assert!(world.contains::<Scheduler>());
    }

    #[test]
    fn multiple_plugins_compose() {
        struct PluginB;
        impl Plugin for PluginB {
            fn build(&self, world: &mut WorldBuilder, scheduler: &mut SchedulerBuilder) {
                scheduler.add_system(sys_b, world.registry_mut());
            }
        }

        let mut app = App::new();
        app.add_plugin(&TestPlugin);
        app.add_plugin(&PluginB);
        let mut world = app.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        assert_eq!(*world.resource::<ExecLog>(), vec![0, 1]);
    }

    #[test]
    fn plugin_ordering_across_plugins() {
        // Systems from plugin A run before systems from plugin B
        // by default (registration order).
        struct PluginA;
        impl Plugin for PluginA {
            fn build(&self, world: &mut WorldBuilder, scheduler: &mut SchedulerBuilder) {
                world.register_default::<ExecLog>();
                scheduler.add_system(sys_a, world.registry_mut());
            }
        }

        struct PluginB;
        impl Plugin for PluginB {
            fn build(&self, world: &mut WorldBuilder, scheduler: &mut SchedulerBuilder) {
                scheduler.add_system(sys_b, world.registry_mut());
            }
        }

        let mut app = App::new();
        app.add_plugin(&PluginA);
        app.add_plugin(&PluginB);
        let mut world = app.build();

        world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w));
        // PluginA's sys_a (push 0) runs before PluginB's sys_b (push 1).
        assert_eq!(*world.resource::<ExecLog>(), vec![0, 1]);
    }
}
