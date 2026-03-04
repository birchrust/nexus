//! Handler combinators for fan-out dispatch.
//!
//! Combinators compose multiple handlers into a single [`Handler`]
//! that dispatches the same event to all of them by reference.
//!
//! - [`FanOut<T>`] — static fan-out. `T` is a tuple of handlers,
//!   each receiving `&E`. Macro-generated for arities 2-8. Zero
//!   allocation, concrete types, monomorphizes to direct calls.
//! - [`Broadcast<E>`] — dynamic fan-out. Stores `Vec<Box<dyn ...>>`
//!   handlers. One heap allocation per handler, zero clones at
//!   dispatch.
//!
//! Both combinators implement `Handler<E>` — they take ownership of
//! the event, borrow it, and forward `&E` to each child handler.
//!
//! Handlers inside combinators must implement `for<'e> Handler<&'e E>`
//! — they receive the event by reference. Use [`Cloned`](crate::Cloned)
//! or [`Owned`](crate::Owned) to adapt owned-event handlers.
//!
//! # Examples
//!
//! ```
//! use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler};
//! use nexus_rt::{fan_out, Broadcast, Cloned};
//!
//! fn write_a(mut sink: ResMut<u64>, event: &u32) {
//!     *sink += *event as u64;
//! }
//!
//! fn write_b(mut sink: ResMut<i64>, event: &u32) {
//!     *sink += *event as i64;
//! }
//!
//! let mut builder = WorldBuilder::new();
//! builder.register::<u64>(0);
//! builder.register::<i64>(0);
//! let mut world = builder.build();
//!
//! // Static 2-way fan-out
//! let h1 = write_a.into_handler(world.registry());
//! let h2 = write_b.into_handler(world.registry());
//! let mut fan = fan_out!(h1, h2);
//! fan.run(&mut world, 5u32);
//! assert_eq!(*world.resource::<u64>(), 5);
//! assert_eq!(*world.resource::<i64>(), 5);
//! ```

use crate::Handler;
use crate::world::World;

// =============================================================================
// fan_out! macro
// =============================================================================

/// Constructs a [`FanOut`] combinator from 2-8 handlers.
///
/// Syntactic sugar for `FanOut((h1, h2, ...))` — avoids the
/// double-parentheses of tuple struct construction.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, fan_out};
///
/// fn inc(mut n: ResMut<u64>, event: &u32) { *n += *event as u64; }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
/// let mut world = builder.build();
///
/// let h1 = inc.into_handler(world.registry());
/// let h2 = inc.into_handler(world.registry());
/// let mut fan = fan_out!(h1, h2);
/// fan.run(&mut world, 1u32);
/// assert_eq!(*world.resource::<u64>(), 2);
/// ```
#[macro_export]
macro_rules! fan_out {
    ($handler:expr $(,)?) => {
        compile_error!("fan_out! requires at least 2 handlers");
    };
    ($($handler:expr),+ $(,)?) => {
        $crate::FanOut(($($handler,)+))
    };
}

// =============================================================================
// FanOut<T> — static tuple fan-out
// =============================================================================

/// Static fan-out combinator. Takes ownership of an event, borrows it,
/// and dispatches `&E` to N handlers.
///
/// `T` is a tuple of handlers — construct via the [`fan_out!`] macro
/// or directly: `FanOut((a, b))`. Macro-generated [`Handler<E>`]
/// impls for tuple arities 2 through 8.
///
/// Each handler in the tuple must implement `for<'e> Handler<&'e E>`.
/// To include an owned-event handler, wrap it in
/// [`Cloned`](crate::Cloned) or [`Owned`](crate::Owned).
///
/// Zero allocation, concrete types — monomorphizes to direct calls.
/// Boxes into `Box<dyn Handler<E>>` for type-erased storage.
///
/// For dynamic fan-out (runtime-determined handler count), use
/// [`Broadcast`].
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, FanOut, Cloned};
///
/// fn ref_handler(mut sink: ResMut<u64>, event: &u32) {
///     *sink += *event as u64;
/// }
///
/// fn owned_handler(mut sink: ResMut<u64>, event: u32) {
///     *sink += event as u64 * 10;
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
/// let mut world = builder.build();
///
/// // Mix ref and owned handlers via Cloned adapter
/// let h1 = ref_handler.into_handler(world.registry());
/// let h2 = owned_handler.into_handler(world.registry());
/// let mut fan = FanOut((h1, Cloned(h2)));
/// fan.run(&mut world, 3u32);
/// assert_eq!(*world.resource::<u64>(), 33); // 3 + 30
/// ```
pub struct FanOut<T>(pub T);

macro_rules! impl_fanout {
    ($($idx:tt: $H:ident),+) => {
        impl<E, $($H),+> Handler<E> for FanOut<($($H,)+)>
        where
            $($H: for<'e> Handler<&'e E> + Send,)+
        {
            fn run(&mut self, world: &mut World, event: E) {
                $(self.0.$idx.run(world, &event);)+
            }

            fn name(&self) -> &'static str {
                "FanOut"
            }
        }
    };
}

impl_fanout!(0: H0, 1: H1);
impl_fanout!(0: H0, 1: H1, 2: H2);
impl_fanout!(0: H0, 1: H1, 2: H2, 3: H3);
impl_fanout!(0: H0, 1: H1, 2: H2, 3: H3, 4: H4);
impl_fanout!(0: H0, 1: H1, 2: H2, 3: H3, 4: H4, 5: H5);
impl_fanout!(0: H0, 1: H1, 2: H2, 3: H3, 4: H4, 5: H5, 6: H6);
impl_fanout!(0: H0, 1: H1, 2: H2, 3: H3, 4: H4, 5: H5, 6: H6, 7: H7);

// =============================================================================
// Broadcast<E> — dynamic fan-out
// =============================================================================

/// Object-safe helper trait that erases the HRTB lifetime from
/// `for<'e> Handler<&'e E>`.
///
/// Rust does not allow `Box<dyn for<'a> Handler<&'a E>>` directly.
/// This trait bridges the gap: any `H: for<'e> Handler<&'e E>`
/// gets a blanket `RefHandler<E>` impl, and [`Broadcast`] stores
/// `Box<dyn RefHandler<E>>`.
///
/// Only `run_ref` is needed — [`Broadcast::name`] returns a fixed
/// string since it wraps N heterogeneous handlers.
trait RefHandler<E>: Send {
    fn run_ref(&mut self, world: &mut World, event: &E);
}

impl<E, H> RefHandler<E> for H
where
    H: for<'e> Handler<&'e E> + Send,
{
    fn run_ref(&mut self, world: &mut World, event: &E) {
        self.run(world, event);
    }
}

/// Dynamic fan-out combinator. Takes ownership of an event, borrows
/// it, and dispatches `&E` to N handlers, where N is determined at
/// runtime.
///
/// One heap allocation per handler (boxing). Zero clones at dispatch
/// — each handler receives `&E`.
///
/// Handlers must implement `for<'e> Handler<&'e E>`. Use
/// [`Cloned`](crate::Cloned) or [`Owned`](crate::Owned) to adapt
/// owned-event handlers.
///
/// For static fan-out (known handler count, zero allocation), use
/// [`FanOut`].
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, Broadcast};
///
/// fn write_a(mut sink: ResMut<u64>, event: &u32) {
///     *sink += *event as u64;
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
/// let mut world = builder.build();
///
/// let mut broadcast: Broadcast<u32> = Broadcast::new();
/// broadcast.add(write_a.into_handler(world.registry()));
/// broadcast.add(write_a.into_handler(world.registry()));
/// broadcast.run(&mut world, 5u32);
/// assert_eq!(*world.resource::<u64>(), 10);
/// ```
pub struct Broadcast<E> {
    handlers: Vec<Box<dyn RefHandler<E>>>,
}

impl<E> Default for Broadcast<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> Broadcast<E> {
    /// Create an empty broadcast with no handlers.
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    /// Add a handler to the broadcast.
    pub fn add<H: for<'e> Handler<&'e E> + Send + 'static>(&mut self, handler: H) {
        self.handlers.push(Box::new(handler));
    }

    /// Returns the number of handlers.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Returns `true` if there are no handlers.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl<E> Handler<E> for Broadcast<E> {
    fn run(&mut self, world: &mut World, event: E) {
        for h in &mut self.handlers {
            h.run_ref(world, &event);
        }
    }

    fn name(&self) -> &'static str {
        "Broadcast"
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cloned, IntoHandler, ResMut, WorldBuilder};

    fn write_u64(mut sink: ResMut<u64>, event: &u32) {
        *sink += *event as u64;
    }

    fn write_i64(mut sink: ResMut<i64>, event: &u32) {
        *sink += *event as i64 * 2;
    }

    fn write_f64(mut sink: ResMut<f64>, event: &u32) {
        *sink += *event as f64 * 0.5;
    }

    fn owned_handler(mut sink: ResMut<u64>, event: u32) {
        *sink += event as u64 * 10;
    }

    // -- FanOut ---------------------------------------------------------------

    #[test]
    fn fanout_two_way() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        builder.register::<i64>(0);
        let mut world = builder.build();

        let h1 = write_u64.into_handler(world.registry());
        let h2 = write_i64.into_handler(world.registry());
        let mut fan = fan_out!(h1, h2);
        fan.run(&mut world, 5u32);
        assert_eq!(*world.resource::<u64>(), 5);
        assert_eq!(*world.resource::<i64>(), 10);
    }

    #[test]
    fn fanout_three_way() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        builder.register::<i64>(0);
        builder.register::<f64>(0.0);
        let mut world = builder.build();

        let h1 = write_u64.into_handler(world.registry());
        let h2 = write_i64.into_handler(world.registry());
        let h3 = write_f64.into_handler(world.registry());
        let mut fan = fan_out!(h1, h2, h3);
        fan.run(&mut world, 10u32);
        assert_eq!(*world.resource::<u64>(), 10);
        assert_eq!(*world.resource::<i64>(), 20);
        assert_eq!(*world.resource::<f64>(), 5.0);
    }

    #[test]
    fn fanout_with_cloned_adapter() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let ref_h = write_u64.into_handler(world.registry());
        let owned_h = owned_handler.into_handler(world.registry());
        let mut fan = fan_out!(ref_h, Cloned(owned_h));
        fan.run(&mut world, 3u32);
        assert_eq!(*world.resource::<u64>(), 33); // 3 + 30
    }

    #[test]
    fn fanout_boxable() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        builder.register::<i64>(0);
        let mut world = builder.build();

        let h1 = write_u64.into_handler(world.registry());
        let h2 = write_i64.into_handler(world.registry());
        let mut boxed: Box<dyn Handler<u32>> = Box::new(fan_out!(h1, h2));
        boxed.run(&mut world, 7u32);
        assert_eq!(*world.resource::<u64>(), 7);
        assert_eq!(*world.resource::<i64>(), 14);
    }

    // -- Broadcast ------------------------------------------------------------

    #[test]
    fn broadcast_dispatch() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let mut broadcast: Broadcast<u32> = Broadcast::new();
        broadcast.add(write_u64.into_handler(world.registry()));
        broadcast.add(write_u64.into_handler(world.registry()));
        broadcast.add(write_u64.into_handler(world.registry()));
        broadcast.run(&mut world, 4u32);
        assert_eq!(*world.resource::<u64>(), 12); // 4 + 4 + 4
    }

    #[test]
    fn broadcast_empty() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let mut broadcast: Broadcast<u32> = Broadcast::new();
        assert!(broadcast.is_empty());
        broadcast.run(&mut world, 1u32);
        assert_eq!(*world.resource::<u64>(), 0);
    }

    #[test]
    fn broadcast_len() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let world = builder.build();

        let mut broadcast: Broadcast<u32> = Broadcast::new();
        assert_eq!(broadcast.len(), 0);
        broadcast.add(write_u64.into_handler(world.registry()));
        assert_eq!(broadcast.len(), 1);
        broadcast.add(write_u64.into_handler(world.registry()));
        assert_eq!(broadcast.len(), 2);
    }

    #[test]
    fn broadcast_with_cloned_adapter() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let mut broadcast: Broadcast<u32> = Broadcast::new();
        broadcast.add(write_u64.into_handler(world.registry()));
        broadcast.add(Cloned(owned_handler.into_handler(world.registry())));
        broadcast.run(&mut world, 2u32);
        assert_eq!(*world.resource::<u64>(), 22); // 2 + 20
    }
}
