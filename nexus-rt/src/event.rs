//! Event buffer types for inter-system communication.
//!
//! [`Events<T>`] is a simple buffer registered as a resource in [`World`].
//! Systems read and write events through [`EventWriter<T>`] (single writer,
//! exclusive access) and [`EventReader<T>`] (multiple readers, shared access).
//!
//! Clearing is the runtime/driver's responsibility — call
//! [`Events::clear`] or [`Events::drain`] between dispatch cycles.

use std::slice;

use crate::resource::{Res, ResMut};
use crate::system::SystemParam;
use crate::world::{Registry, ResourceId, World};

// =============================================================================
// Events<T>
// =============================================================================

/// Simple Vec-based event buffer. Registered as a resource in [`World`].
///
/// The runtime decides when events are cleared — this type only provides
/// storage and access.
pub struct Events<T> {
    buffer: Vec<T>,
}

impl<T> Events<T> {
    /// Create an empty event buffer.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Push an event into the buffer.
    pub fn send(&mut self, event: T) {
        self.buffer.push(event);
    }

    /// Consume all events, returning a draining iterator.
    pub fn drain(&mut self) -> std::vec::Drain<'_, T> {
        self.buffer.drain(..)
    }

    /// Discard all events.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Iterate over events without consuming them.
    pub fn iter(&self) -> slice::Iter<'_, T> {
        self.buffer.iter()
    }

    /// Returns the number of pending events.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Returns `true` if no events are pending.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T> IntoIterator for &'a Events<T> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// =============================================================================
// EventWriter<T>
// =============================================================================

/// System parameter for writing events into an [`Events<T>`] buffer.
///
/// Wraps [`ResMut<Events<T>>`] — exclusive access. One writer per
/// dispatch cycle.
pub struct EventWriter<'w, T: 'static> {
    events: ResMut<'w, Events<T>>,
}

impl<T: 'static> EventWriter<'_, T> {
    /// Push an event into the buffer.
    pub fn send(&mut self, event: T) {
        self.events.send(event);
    }
}

impl<T: 'static> SystemParam for EventWriter<'_, T> {
    type State = ResourceId;
    type Item<'w> = EventWriter<'w, T>;

    fn init(registry: &Registry) -> ResourceId {
        registry.id::<Events<T>>()
    }

    #[inline(always)]
    unsafe fn fetch<'w>(world: &'w World, state: &'w mut ResourceId) -> EventWriter<'w, T> {
        let id = *state;
        // SAFETY: state was produced by init() on the same world.
        // Caller ensures no aliases exist for Events<T>.
        EventWriter {
            events: unsafe {
                ResMut::new(
                    world.get_mut::<Events<T>>(id),
                    world.changed_at_cell(id),
                    world.current_tick(),
                )
            },
        }
    }

    fn any_changed(state: &ResourceId, world: &World) -> bool {
        // SAFETY: state was produced by init() on the same registry.
        unsafe { world.changed_at(*state) == world.current_tick() }
    }

    fn resource_id(state: &ResourceId) -> Option<ResourceId> {
        Some(*state)
    }
}

// =============================================================================
// EventReader<T>
// =============================================================================

/// System parameter for reading events from an [`Events<T>`] buffer.
///
/// Wraps [`Res<Events<T>>`] — shared access. Multiple readers can
/// coexist in the same dispatch cycle.
pub struct EventReader<'w, T: 'static> {
    events: Res<'w, Events<T>>,
}

impl<T: 'static> EventReader<'_, T> {
    /// Iterate over pending events.
    pub fn iter(&self) -> slice::Iter<'_, T> {
        self.events.iter()
    }

    /// Returns `true` if no events are pending.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Returns the number of pending events.
    pub fn len(&self) -> usize {
        self.events.len()
    }
}

impl<'a, T: 'static> IntoIterator for &'a EventReader<'_, T> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: 'static> SystemParam for EventReader<'_, T> {
    type State = ResourceId;
    type Item<'w> = EventReader<'w, T>;

    fn init(registry: &Registry) -> ResourceId {
        registry.id::<Events<T>>()
    }

    #[inline(always)]
    unsafe fn fetch<'w>(world: &'w World, state: &'w mut ResourceId) -> EventReader<'w, T> {
        let id = *state;
        // SAFETY: state was produced by init() on the same world.
        // Caller ensures no mutable alias exists for Events<T>.
        EventReader {
            events: unsafe {
                Res::new(
                    world.get::<Events<T>>(id),
                    world.changed_at(id),
                    world.current_tick(),
                )
            },
        }
    }

    fn any_changed(state: &ResourceId, world: &World) -> bool {
        // SAFETY: state was produced by init() on the same registry.
        unsafe { world.changed_at(*state) == world.current_tick() }
    }

    fn resource_id(state: &ResourceId) -> Option<ResourceId> {
        Some(*state)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoSystem, System, WorldBuilder};

    #[test]
    fn events_send_and_drain() {
        let mut events = Events::new();
        events.send(1u32);
        events.send(2);
        events.send(3);
        assert_eq!(events.len(), 3);

        let drained: Vec<u32> = events.drain().collect();
        assert_eq!(drained, vec![1, 2, 3]);
        assert!(events.is_empty());
    }

    #[test]
    fn event_writer_system_param() {
        let mut builder = WorldBuilder::new();
        builder.register::<Events<u32>>(Events::new());
        let world = builder.build();

        let mut state = <EventWriter<u32> as SystemParam>::init(world.registry());
        // SAFETY: state from init on same registry, no aliasing.
        unsafe {
            let mut writer = <EventWriter<u32> as SystemParam>::fetch(&world, &mut state);
            writer.send(42);
            writer.send(99);
        }
        unsafe {
            let events = world.get::<Events<u32>>(world.id::<Events<u32>>());
            assert_eq!(events.len(), 2);
            let vals: Vec<&u32> = events.iter().collect();
            assert_eq!(vals, vec![&42, &99]);
        }
    }

    #[test]
    fn event_reader_system_param() {
        let mut events = Events::new();
        events.send(10u32);
        events.send(20);

        let mut builder = WorldBuilder::new();
        builder.register::<Events<u32>>(events);
        let world = builder.build();

        let mut state = <EventReader<u32> as SystemParam>::init(world.registry());
        // SAFETY: state from init on same registry, no aliasing.
        let reader = unsafe { <EventReader<u32> as SystemParam>::fetch(&world, &mut state) };
        assert_eq!(reader.len(), 2);
        let vals: Vec<&u32> = reader.iter().collect();
        assert_eq!(vals, vec![&10, &20]);
    }

    fn write_events(mut writer: EventWriter<u32>, tick: u32) {
        writer.send(tick);
        writer.send(tick * 10);
    }

    fn read_events(reader: EventReader<u32>, expected_count: usize) {
        assert_eq!(reader.len(), expected_count);
    }

    #[test]
    fn writer_then_reader_in_systems() {
        let mut builder = WorldBuilder::new();
        builder.register::<Events<u32>>(Events::new());
        let mut world = builder.build();

        let mut writer_sys = write_events.into_system(world.registry_mut());
        let mut reader_sys = read_events.into_system(world.registry_mut());

        writer_sys.run(&mut world, 5u32);
        reader_sys.run(&mut world, 2usize);
    }

    // -- Change detection tests -----------------------------------------------

    #[test]
    fn inputs_changed_with_events() {
        let mut builder = WorldBuilder::new();
        builder.register_default::<Events<u32>>();
        let mut world = builder.build();

        let writer_sys = write_events.into_system(world.registry_mut());
        let reader_sys = read_events.into_system(world.registry_mut());

        // Tick 0: Events registered at tick=0, current_tick=0 → changed
        assert!(writer_sys.inputs_changed(&world));
        assert!(reader_sys.inputs_changed(&world));

        world.advance_tick(); // tick=1

        // Tick 1: Events changed_at=0, current_tick=1 → not changed
        assert!(!writer_sys.inputs_changed(&world));
        assert!(!reader_sys.inputs_changed(&world));
    }

    // -- Access conflict detection -------------------------------------------

    #[test]
    #[should_panic(expected = "conflicting access")]
    fn duplicate_event_access_panics() {
        let mut builder = WorldBuilder::new();
        builder.register_default::<Events<u32>>();
        let mut world = builder.build();

        fn bad(w: EventWriter<u32>, r: EventReader<u32>, _e: ()) {
            let _ = (w, r);
        }

        let _sys = bad.into_system(world.registry_mut());
    }

    #[test]
    fn event_writer_stamps_events() {
        let mut builder = WorldBuilder::new();
        builder.register_default::<Events<u32>>();
        let mut world = builder.build();

        world.advance_tick(); // tick=1

        let events_id = world.id::<Events<u32>>();
        // Events registered at tick=0, now at tick=1 → not changed
        unsafe {
            assert_ne!(world.changed_at(events_id), world.current_tick());
        }

        // EventWriter::send → DerefMut on inner ResMut → stamps
        let mut writer_sys = write_events.into_system(world.registry_mut());
        writer_sys.run(&mut world, 1u32);

        unsafe {
            assert_eq!(world.changed_at(events_id), world.current_tick());
        }
    }
}
