//! Event-type adapters for handlers.
//!
//! Each adapter wraps a single handler and transforms its event interface.
//!
//! - [`Adapt`] — decodes a wire event into a domain type, skipping
//!   dispatch on `None`.
//! - [`ByRef`] — wraps a `Handler<&E>` to implement `Handler<E>`.
//!   The event is borrowed before dispatch.
//! - [`Cloned`] — wraps a `Handler<E>` to implement `Handler<&E>`.
//!   The event is cloned before dispatch. Explicit opt-in to the
//!   clone cost.
//! - [`Owned`] — wraps a `Handler<E::Owned>` to implement `Handler<&E>`
//!   via [`ToOwned`]. More general than `Cloned`: handles `&str → String`,
//!   `&[u8] → Vec<u8>`, etc.

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

// =============================================================================
// ByRef — owned-to-reference adapter
// =============================================================================

/// Owned-to-reference adapter. Wraps a [`Handler<&E>`](Handler) and
/// implements `Handler<E>` — the event is borrowed before dispatch.
///
/// Use when a handler written for `&E` needs to slot into a position
/// that provides owned `E`. This is the natural adapter for handlers
/// inside [`FanOut`](crate::FanOut) or [`Broadcast`](crate::Broadcast)
/// that were originally written for owned events.
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, ByRef};
///
/// fn process(mut counter: ResMut<u64>, event: &u32) {
///     *counter += *event as u64;
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
/// let mut world = builder.build();
///
/// let h = process.into_handler(world.registry());
/// let mut adapted = ByRef(h);
/// adapted.run(&mut world, 5u32);
/// assert_eq!(*world.resource::<u64>(), 5);
/// ```
pub struct ByRef<H>(pub H);

impl<E, H> Handler<E> for ByRef<H>
where
    H: for<'e> Handler<&'e E> + Send,
{
    fn run(&mut self, world: &mut World, event: E) {
        self.0.run(world, &event);
    }

    fn name(&self) -> &'static str {
        self.0.name()
    }
}

// =============================================================================
// Cloned — reference-to-owned adapter
// =============================================================================

/// Reference-to-owned adapter. Wraps a [`Handler<E>`](Handler) and
/// implements `Handler<&E>` — the event is cloned before dispatch.
///
/// Explicit opt-in to the clone cost. For `E: Copy` the compiler
/// elides the clone entirely.
///
/// Use when an owned-event handler needs to participate in a
/// reference-based dispatch context (e.g. inside a
/// [`FanOut`](crate::FanOut)).
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, Cloned};
///
/// fn process(mut counter: ResMut<u64>, event: u32) {
///     *counter += event as u64;
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(0);
/// let mut world = builder.build();
///
/// let h = process.into_handler(world.registry());
/// let mut adapted = Cloned(h);
/// adapted.run(&mut world, &5u32);
/// assert_eq!(*world.resource::<u64>(), 5);
/// ```
pub struct Cloned<H>(pub H);

impl<'e, E: Clone + 'e, H: Handler<E> + Send> Handler<&'e E> for Cloned<H> {
    fn run(&mut self, world: &mut World, event: &'e E) {
        self.0.run(world, event.clone());
    }

    fn name(&self) -> &'static str {
        self.0.name()
    }
}

// =============================================================================
// Owned — reference-to-owned adapter via ToOwned
// =============================================================================

/// Reference-to-owned adapter via [`ToOwned`]. Wraps a
/// [`Handler<E::Owned>`](Handler) and implements `Handler<&E>` — the
/// event is converted via [`to_owned()`](ToOwned::to_owned) before
/// dispatch.
///
/// More general than [`Cloned`]: handles `&str → String`,
/// `&[u8] → Vec<u8>`, and any other [`ToOwned`] impl where the owned
/// type differs from the reference target. For `T: Clone`, `ToOwned`
/// is blanket-implemented with `Owned = T`, so this adapter also
/// works as a drop-in replacement for `Cloned` in those cases.
///
/// `E` must be named explicitly because the `ToOwned` mapping is not
/// invertible — given `Handler<String>`, the compiler cannot infer
/// that `E = str`. Use [`Owned::new`] with turbofish when needed.
/// For simple `Clone` types where `E = E::Owned`, prefer [`Cloned`].
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, Owned};
///
/// fn process(mut buf: ResMut<String>, event: String) {
///     buf.push_str(&event);
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<String>(String::new());
/// let mut world = builder.build();
///
/// let h = process.into_handler(world.registry());
/// let mut adapted = Owned::<_, str>::new(h);
/// adapted.run(&mut world, "hello");
/// assert_eq!(world.resource::<String>().as_str(), "hello");
/// ```
pub struct Owned<H, E: ?Sized> {
    handler: H,
    _event: std::marker::PhantomData<fn(&E)>,
}

impl<H, E: ?Sized> Owned<H, E> {
    /// Create a new `Owned` adapter.
    ///
    /// When `E` cannot be inferred, use turbofish:
    /// `Owned::<_, str>::new(handler)`.
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            _event: std::marker::PhantomData,
        }
    }
}

impl<'e, E, H> Handler<&'e E> for Owned<H, E>
where
    E: ToOwned + 'e + ?Sized,
    H: Handler<E::Owned> + Send,
{
    fn run(&mut self, world: &mut World, event: &'e E) {
        self.handler.run(world, event.to_owned());
    }

    fn name(&self) -> &'static str {
        self.handler.name()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoHandler, ResMut, WorldBuilder};

    struct WireMsg(u32);

    // Wire type is &WireMsg — taken by value (already a reference).
    #[allow(clippy::unnecessary_wraps)]
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

    // -- ByRef ----------------------------------------------------------------

    fn ref_accumulate(mut counter: ResMut<u64>, event: &u64) {
        *counter += *event;
    }

    #[test]
    fn by_ref_dispatch() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let h = ref_accumulate.into_handler(world.registry());
        let mut adapted = ByRef(h);
        adapted.run(&mut world, 10u64);
        adapted.run(&mut world, 5u64);
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn by_ref_delegates_name() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let world = builder.build();

        let handler = ref_accumulate.into_handler(world.registry());
        let expected = handler.name();
        let adapted = ByRef(handler);
        assert_eq!(adapted.name(), expected);
    }

    // -- Cloned ---------------------------------------------------------------

    #[test]
    fn cloned_dispatch() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let h = accumulate.into_handler(world.registry());
        let mut adapted = Cloned(h);
        adapted.run(&mut world, &10u64);
        adapted.run(&mut world, &5u64);
        assert_eq!(*world.resource::<u64>(), 15);
    }

    #[test]
    fn cloned_delegates_name() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let world = builder.build();

        let handler = accumulate.into_handler(world.registry());
        let expected = handler.name();
        let adapted = Cloned(handler);
        assert_eq!(adapted.name(), expected);
    }

    #[test]
    fn cloned_copy_type() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        // u32 is Copy — clone is free
        fn add_u32(mut counter: ResMut<u64>, event: u32) {
            *counter += event as u64;
        }

        let h = add_u32.into_handler(world.registry());
        let mut adapted = Cloned(h);
        adapted.run(&mut world, &42u32);
        assert_eq!(*world.resource::<u64>(), 42);
    }

    // -- Owned ----------------------------------------------------------------

    fn append_string(mut buf: ResMut<String>, event: String) {
        buf.push_str(&event);
    }

    #[test]
    fn owned_str_to_string() {
        let mut builder = WorldBuilder::new();
        builder.register::<String>(String::new());
        let mut world = builder.build();

        let h = append_string.into_handler(world.registry());
        let mut adapted = Owned::<_, str>::new(h);
        // &str → String via ToOwned
        adapted.run(&mut world, "hello");
        adapted.run(&mut world, " world");
        assert_eq!(world.resource::<String>().as_str(), "hello world");
    }

    #[test]
    fn owned_delegates_name() {
        let mut builder = WorldBuilder::new();
        builder.register::<String>(String::new());
        let world = builder.build();

        let handler = append_string.into_handler(world.registry());
        let expected = handler.name();
        let adapted = Owned::<_, str>::new(handler);
        assert_eq!(adapted.name(), expected);
    }

    #[test]
    fn owned_clone_type() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        // u64: Clone, so ToOwned blanket impl gives Owned = u64
        let h = accumulate.into_handler(world.registry());
        let mut adapted = Owned::<_, u64>::new(h);
        adapted.run(&mut world, &10u64);
        adapted.run(&mut world, &5u64);
        assert_eq!(*world.resource::<u64>(), 15);
    }
}
