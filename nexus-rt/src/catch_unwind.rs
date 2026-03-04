//! Panic-catching annotation for handlers.

use std::panic::AssertUnwindSafe;

use crate::handler::Handler;
use crate::world::World;

/// Panic-catching wrapper for [`Handler`] implementations.
///
/// Catches panics during [`run()`](Handler::run) so the handler is never
/// lost during move-out-fire dispatch. This is an annotation — wrap a
/// concrete handler, then virtualize through your chosen storage (`Box`,
/// `Flat`, `Flex`, typed slab, etc.).
///
/// By constructing this wrapper, the user asserts that the inner handler
/// (and any [`World`] resources it mutates) can tolerate partial writes
/// caused by an unwound `run()` call. This is the same assertion as
/// [`std::panic::AssertUnwindSafe`], applied at the handler level.
///
/// # Examples
///
/// ```
/// use nexus_rt::{CatchAssertUnwindSafe, WorldBuilder, ResMut, IntoHandler, Handler, Virtual};
///
/// fn tick(mut counter: ResMut<u64>, event: u32) {
///     *counter += event as u64;
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
/// let mut world = builder.build();
///
/// let handler = tick.into_handler(world.registry());
/// let guarded = CatchAssertUnwindSafe::new(handler);
/// let mut boxed: Virtual<u32> = Box::new(guarded);
///
/// boxed.run(&mut world, 10);
/// assert_eq!(*world.resource::<u64>(), 10);
/// ```
pub struct CatchAssertUnwindSafe<H> {
    handler: H,
}

impl<H> CatchAssertUnwindSafe<H> {
    /// Wrap a handler with panic catching.
    ///
    /// The caller asserts that the handler and any resources it touches
    /// are safe to continue using after a caught panic.
    pub fn new(handler: H) -> Self {
        Self { handler }
    }
}

impl<E, H: Handler<E>> Handler<E> for CatchAssertUnwindSafe<H> {
    fn run(&mut self, world: &mut World, event: E) {
        let handler = &mut self.handler;
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            handler.run(world, event);
        }));
    }

    fn name(&self) -> &'static str {
        self.handler.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoHandler, ResMut, WorldBuilder};

    fn normal_handler(mut val: ResMut<u64>, event: u64) {
        *val += event;
    }

    #[test]
    fn forwards_run() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let handler = normal_handler.into_handler(world.registry());
        let mut guarded = CatchAssertUnwindSafe::new(handler);

        guarded.run(&mut world, 10);
        assert_eq!(*world.resource::<u64>(), 10);
    }

    fn panicking_handler(_val: ResMut<u64>, _event: u64) {
        panic!("boom");
    }

    #[test]
    fn survives_panic() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let handler = panicking_handler.into_handler(world.registry());
        let mut guarded = CatchAssertUnwindSafe::new(handler);

        // Should not panic — caught internally.
        guarded.run(&mut world, 10);

        // Handler survives — can be called again.
        guarded.run(&mut world, 10);
    }

    #[test]
    fn forwards_name() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let world = builder.build();

        let handler = normal_handler.into_handler(world.registry());
        let guarded = CatchAssertUnwindSafe::new(handler);

        assert!(guarded.name().contains("normal_handler"));
    }
}
