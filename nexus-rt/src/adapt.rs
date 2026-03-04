//! Event adapter — decodes a wire event and dispatches to an inner handler.

use crate::Handler;
use crate::world::World;

/// Lightweight adapter that decodes a wire-format event into a domain type
/// before dispatching to an inner handler.
///
/// Implements [`Handler<Wire>`] by calling `decode(Wire) -> Option<T>`,
/// then forwarding `T` to the inner [`Handler<T>`]. Skips dispatch when
/// decode returns `None` (wrong template, decode error, filtered, etc.).
///
/// The decode function takes `Wire` by value. For reference types like
/// SBE flyweight decoders (`ReadBuf<'a>`), this is already a borrow -
/// no double indirection.
///
/// Both the decode function and inner handler are concrete types —
/// monomorphizes to a direct call chain with no vtable overhead.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler};
/// use nexus_rt::Adapt;
///
/// // Wire event — in practice this would be a decoder/buffer type.
/// struct WireMsg(u32);
///
/// // Decode takes Wire by value. For reference-type wire events
/// // (e.g. SBE flyweight decoders like MessageHeaderDecoder<ReadBuf<'a>>),
/// // this is already a borrow — no double indirection.
/// fn decode_wire(wire: &WireMsg) -> Option<u64> {
///     Some(wire.0 as u64)
/// }
///
/// fn accumulate(mut counter: ResMut<u64>, event: u64) {
///     *counter += event;
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
/// let mut world = builder.build();
///
/// let handler = accumulate.into_handler(world.registry());
/// let mut adapted: Adapt<_, _> = Adapt::new(decode_wire, handler);
///
/// // Wire type is &WireMsg — reference type taken by value.
/// adapted.run(&mut world, &WireMsg(10));
/// adapted.run(&mut world, &WireMsg(5));
/// assert_eq!(*world.resource::<u64>(), 15);
/// ```
pub struct Adapt<F, H> {
    decode: F,
    inner: H,
}

impl<F, H> Adapt<F, H> {
    /// Create a new adapter from a decode function and an inner handler.
    pub fn new(decode: F, inner: H) -> Self {
        Self { decode, inner }
    }
}

impl<Wire, T, F, H> Handler<Wire> for Adapt<F, H>
where
    F: FnMut(Wire) -> Option<T> + Send,
    H: Handler<T>,
{
    fn run(&mut self, world: &mut World, event: Wire) {
        if let Some(decoded) = (self.decode)(event) {
            self.inner.run(world, decoded);
        }
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoHandler, ResMut, WorldBuilder};

    struct WireMsg(u32);

    // Wire type is &WireMsg — taken by value (already a reference).
    fn decode_wire(wire: &WireMsg) -> Option<u64> {
        Some(wire.0 as u64)
    }

    fn decode_filter(wire: &WireMsg) -> Option<u64> {
        if wire.0 > 0 {
            Some(wire.0 as u64)
        } else {
            None
        }
    }

    fn accumulate(mut counter: ResMut<u64>, event: u64) {
        *counter += event;
    }

    #[test]
    fn dispatch_decodes_and_forwards() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let handler = accumulate.into_handler(world.registry());
        let mut adapted = Adapt::new(decode_wire, handler);

        adapted.run(&mut world, &WireMsg(10));
        adapted.run(&mut world, &WireMsg(5));
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn none_skips_dispatch() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let handler = accumulate.into_handler(world.registry());
        let mut adapted = Adapt::new(decode_filter, handler);

        adapted.run(&mut world, &WireMsg(10));
        adapted.run(&mut world, &WireMsg(0)); // filtered
        adapted.run(&mut world, &WireMsg(3));
        assert_eq!(*world.resource::<u64>(), 13);
    }

    #[test]
    fn delegates_name() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let world = builder.build();

        let handler = accumulate.into_handler(world.registry());
        let expected = handler.name();
        let adapted = Adapt::new(decode_wire, handler);

        assert_eq!(adapted.name(), expected);
    }
}
