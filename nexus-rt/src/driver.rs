//! Installer trait for event source installation.

use crate::world::WorldBuilder;

/// Install-time trait for event sources.
///
/// An installer registers its resources into [`WorldBuilder`] and returns a
/// concrete poller. The poller is a thin struct of pre-resolved
/// [`ResourceId`](crate::ResourceId)s — it knows how to reach into
/// [`World`](crate::World) but owns nothing.
///
/// Each poller defines its own `poll()` signature. This is intentional:
/// different drivers need different parameters (e.g. a timer driver
/// needs `Instant`, an IO driver does not).
///
/// # Examples
///
/// ```ignore
/// struct IoInstaller { capacity: usize }
///
/// struct IoPoller {
///     poller_id: ResourceId,
///     events_id: ResourceId,
/// }
///
/// impl Installer for IoInstaller {
///     type Poller = IoPoller;
///
///     fn install(self, world: &mut WorldBuilder) -> IoPoller {
///         let poller_id = world.register(Poller::new());
///         let events_id = world.register(MioEvents::with_capacity(self.capacity));
///         IoPoller { poller_id, events_id }
///     }
/// }
///
/// // Poller has its own poll signature — NOT a trait method.
/// impl IoPoller {
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
pub trait Installer {
    /// The concrete poller returned after installation.
    type Poller;

    /// Register resources into the world and return a poller for dispatch.
    fn install(self, world: &mut WorldBuilder) -> Self::Poller;
}
