//! Driver trait for event source installation.

use crate::world::WorldBuilder;

/// Install-time trait for event sources.
///
/// A driver registers its resources into [`WorldBuilder`] and returns a
/// concrete handle. The handle is a thin struct of pre-resolved
/// [`ResourceId`](crate::ResourceId)s — it knows how to reach into
/// [`World`](crate::World) but owns nothing.
///
/// Each handle defines its own `poll()` signature. This is intentional:
/// different drivers need different parameters (e.g. a timer driver
/// needs `Instant`, an IO driver does not).
///
/// # Examples
///
/// ```ignore
/// struct IoInstaller { capacity: usize }
///
/// struct IoHandle {
///     poller_id: ResourceId,
///     events_id: ResourceId,
/// }
///
/// impl Driver for IoInstaller {
///     type Handle = IoHandle;
///
///     fn install(self, world: &mut WorldBuilder) -> IoHandle {
///         world.register(Poller::new());
///         world.register(MioEvents::with_capacity(self.capacity));
///         let poller_id = world.registry().id::<Poller>();
///         let events_id = world.registry().id::<MioEvents>();
///         IoHandle { poller_id, events_id }
///     }
/// }
///
/// // Handle has its own poll signature — NOT a trait method.
/// impl IoHandle {
///     fn poll(&mut self, world: &mut World) {
///         // get resources via pre-resolved IDs, poll mio, dispatch
///     }
/// }
///
/// let mut wb = WorldBuilder::new();
/// let io = wb.install_driver(IoInstaller { capacity: 1024 });
/// let mut world = wb.build();
///
/// loop {
///     io.poll(&mut world);
/// }
/// ```
pub trait Driver {
    /// The concrete handle returned after installation.
    type Handle;

    /// Register resources into the world and return a handle for dispatch.
    fn install(self, world: &mut WorldBuilder) -> Self::Handle;
}
