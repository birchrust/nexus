//! Timer wheel — the main data structure.
//!
//! `TimerWheel<T, S>` is a multi-level, no-cascade timer wheel. Entries are
//! placed into a level based on how far in the future their deadline is.
//! Once placed, an entry never moves — poll checks `deadline_ticks <= now`
//! per entry.

use std::marker::PhantomData;
use std::mem;
use std::time::{Duration, Instant};

use nexus_slab::{Full, bounded, unbounded};

use crate::entry::{EntryPtr, WheelEntry, entry_ref, null_entry};
use crate::handle::TimerHandle;
use crate::level::Level;
use crate::store::{BoundedStore, SlabStore, UnboundedStore};

// =============================================================================
// WheelBuilder (typestate)
// =============================================================================

/// Builder for configuring a timer wheel.
///
/// Defaults match the Linux kernel timer wheel (1ms tick, 64 slots/level,
/// 8x multiplier, 7 levels → ~4.7 hour range).
///
/// # Examples
///
/// ```
/// use std::time::{Duration, Instant};
/// use nexus_timer::{Wheel, WheelBuilder};
///
/// let now = Instant::now();
///
/// // All defaults
/// let wheel: Wheel<u64> = WheelBuilder::default().unbounded(4096).build(now);
///
/// // Custom config
/// let wheel: Wheel<u64> = WheelBuilder::default()
///     .tick_duration(Duration::from_micros(100))
///     .slots_per_level(128)
///     .unbounded(4096)
///     .build(now);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct WheelBuilder {
    tick_duration: Duration,
    slots_per_level: usize,
    clk_shift: u32,
    num_levels: usize,
}

impl Default for WheelBuilder {
    fn default() -> Self {
        WheelBuilder {
            tick_duration: Duration::from_millis(1),
            slots_per_level: 64,
            clk_shift: 3,
            num_levels: 7,
        }
    }
}

impl WheelBuilder {
    /// Creates a new builder with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the tick duration. Default: 1ms.
    pub fn tick_duration(mut self, d: Duration) -> Self {
        self.tick_duration = d;
        self
    }

    /// Sets the number of slots per level. Must be a power of 2. Default: 64.
    pub fn slots_per_level(mut self, n: usize) -> Self {
        self.slots_per_level = n;
        self
    }

    /// Sets the bit shift between levels (multiplier = 2^clk_shift). Default: 3 (8x).
    pub fn clk_shift(mut self, s: u32) -> Self {
        self.clk_shift = s;
        self
    }

    /// Sets the number of levels. Default: 7.
    pub fn num_levels(mut self, n: usize) -> Self {
        self.num_levels = n;
        self
    }

    /// Transitions to an unbounded wheel builder.
    ///
    /// `chunk_capacity` is the slab chunk size (entries per chunk). The slab
    /// grows by adding new chunks as needed.
    pub fn unbounded(self, chunk_capacity: usize) -> UnboundedWheelBuilder {
        UnboundedWheelBuilder {
            config: self,
            chunk_capacity,
        }
    }

    /// Transitions to a bounded wheel builder.
    ///
    /// `capacity` is the maximum number of concurrent timers.
    pub fn bounded(self, capacity: usize) -> BoundedWheelBuilder {
        BoundedWheelBuilder {
            config: self,
            capacity,
        }
    }

    fn validate(&self) {
        assert!(
            self.slots_per_level.is_power_of_two(),
            "slots_per_level must be a power of 2, got {}",
            self.slots_per_level
        );
        assert!(self.num_levels > 0, "num_levels must be > 0");
        assert!(self.clk_shift > 0, "clk_shift must be > 0");
        assert!(
            !self.tick_duration.is_zero(),
            "tick_duration must be non-zero"
        );
    }

    fn tick_ns(&self) -> u64 {
        self.tick_duration.as_nanos() as u64
    }
}

/// Terminal builder for an unbounded timer wheel.
///
/// Created via [`WheelBuilder::unbounded`]. The only method is `.build()`.
#[derive(Debug)]
pub struct UnboundedWheelBuilder {
    config: WheelBuilder,
    chunk_capacity: usize,
}

impl UnboundedWheelBuilder {
    /// Builds the unbounded timer wheel.
    ///
    /// # Panics
    ///
    /// Panics if the configuration is invalid (non-power-of-2 slots, zero
    /// levels, zero clk_shift, or zero tick duration).
    pub fn build<T: 'static>(self, now: Instant) -> Wheel<T> {
        self.config.validate();
        let slab = unbounded::Slab::with_chunk_capacity(self.chunk_capacity);
        let levels = build_levels::<T>(&self.config);
        TimerWheel {
            slab,
            num_levels: self.config.num_levels,
            levels,
            current_ticks: 0,
            tick_ns: self.config.tick_ns(),
            epoch: now,
            len: 0,
            bookmark: PollBookmark::new(),
            _marker: PhantomData,
        }
    }
}

/// Terminal builder for a bounded timer wheel.
///
/// Created via [`WheelBuilder::bounded`]. The only method is `.build()`.
#[derive(Debug)]
pub struct BoundedWheelBuilder {
    config: WheelBuilder,
    capacity: usize,
}

impl BoundedWheelBuilder {
    /// Builds the bounded timer wheel.
    ///
    /// # Panics
    ///
    /// Panics if the configuration is invalid (non-power-of-2 slots, zero
    /// levels, zero clk_shift, or zero tick duration).
    pub fn build<T: 'static>(self, now: Instant) -> BoundedWheel<T> {
        self.config.validate();
        let slab = bounded::Slab::with_capacity(self.capacity);
        let levels = build_levels::<T>(&self.config);
        TimerWheel {
            slab,
            num_levels: self.config.num_levels,
            levels,
            current_ticks: 0,
            tick_ns: self.config.tick_ns(),
            epoch: now,
            len: 0,
            bookmark: PollBookmark::new(),
            _marker: PhantomData,
        }
    }
}

// =============================================================================
// Poll bookmark — for resumable poll_with_limit
// =============================================================================

/// Bookmark for resumable polling.
///
/// Tracks the position within the active-slot walk so `poll_with_limit` can
/// resume where it left off on the next call.
struct PollBookmark<T> {
    /// Current level index being polled.
    level: usize,
    /// Current slot pointer within the active-slot list (null = start of level).
    slot: *mut crate::level::WheelSlot<T>,
    /// Current entry pointer within the slot's entry DLL (null = start of slot).
    entry: EntryPtr<T>,
    /// The tick value this bookmark was created for.
    ticks: u64,
}

impl<T> PollBookmark<T> {
    fn new() -> Self {
        PollBookmark {
            level: 0,
            slot: std::ptr::null_mut(),
            entry: null_entry(),
            ticks: 0,
        }
    }

    fn reset(&mut self) {
        self.level = 0;
        self.slot = std::ptr::null_mut();
        self.entry = null_entry();
        self.ticks = 0;
    }
}

// =============================================================================
// TimerWheel
// =============================================================================

/// A multi-level, no-cascade timer wheel.
///
/// Generic over:
/// - `T` — the user payload stored with each timer.
/// - `S` — the slab storage backend. Defaults to `unbounded::Slab`.
///
/// # Thread Safety
///
/// `!Send`, `!Sync`. Must be used from a single thread.
pub struct TimerWheel<
    T: 'static,
    S: SlabStore<Item = WheelEntry<T>> = unbounded::Slab<WheelEntry<T>>,
> {
    slab: S,
    levels: Vec<Level<T>>,
    num_levels: usize,
    current_ticks: u64,
    tick_ns: u64,
    epoch: Instant,
    len: usize,
    bookmark: PollBookmark<T>,
    _marker: PhantomData<*const ()>, // !Send, !Sync
}

/// A timer wheel backed by a fixed-capacity slab.
pub type BoundedWheel<T> = TimerWheel<T, bounded::Slab<WheelEntry<T>>>;

/// A timer wheel backed by a growable slab.
pub type Wheel<T> = TimerWheel<T, unbounded::Slab<WheelEntry<T>>>;

// =============================================================================
// Construction
// =============================================================================

impl<T: 'static> Wheel<T> {
    /// Creates an unbounded timer wheel with default configuration.
    ///
    /// For custom configuration, use [`WheelBuilder`].
    pub fn unbounded(chunk_capacity: usize, now: Instant) -> Self {
        WheelBuilder::default().unbounded(chunk_capacity).build(now)
    }
}

impl<T: 'static> BoundedWheel<T> {
    /// Creates a bounded timer wheel with default configuration.
    ///
    /// For custom configuration, use [`WheelBuilder`].
    pub fn bounded(capacity: usize, now: Instant) -> Self {
        WheelBuilder::default().bounded(capacity).build(now)
    }
}

fn build_levels<T: 'static>(config: &WheelBuilder) -> Vec<Level<T>> {
    (0..config.num_levels)
        .map(|i| Level::new(config.slots_per_level, i, config.clk_shift))
        .collect()
}

// =============================================================================
// Schedule — unbounded (always succeeds)
// =============================================================================

impl<T: 'static, S: UnboundedStore<Item = WheelEntry<T>> + SlabStore<Item = WheelEntry<T>>>
    TimerWheel<T, S>
{
    /// Schedules a timer and returns a handle for cancellation.
    ///
    /// The handle must be consumed via [`cancel`](Self::cancel) or
    /// [`free`](Self::free). Dropping it is a programming error.
    pub fn schedule(&mut self, deadline: Instant, value: T) -> TimerHandle<T> {
        let deadline_ticks = self.instant_to_ticks(deadline);
        let entry = WheelEntry::new(deadline_ticks, value, 2);
        let slot = self.slab.alloc(entry);
        let ptr = slot.as_ptr();
        self.insert_entry(ptr, deadline_ticks);
        self.len += 1;
        TimerHandle::new(ptr)
    }

    /// Schedules a fire-and-forget timer (no handle returned).
    ///
    /// The timer will fire during poll and the value will be collected.
    /// Cannot be cancelled.
    pub fn schedule_forget(&mut self, deadline: Instant, value: T) {
        let deadline_ticks = self.instant_to_ticks(deadline);
        let entry = WheelEntry::new(deadline_ticks, value, 1);
        let slot = self.slab.alloc(entry);
        let ptr = slot.as_ptr();
        self.insert_entry(ptr, deadline_ticks);
        self.len += 1;
    }
}

// =============================================================================
// Schedule — bounded (can fail)
// =============================================================================

impl<T: 'static, S: BoundedStore<Item = WheelEntry<T>> + SlabStore<Item = WheelEntry<T>>>
    TimerWheel<T, S>
{
    /// Attempts to schedule a timer, returning a handle on success.
    ///
    /// Returns `Err(Full(value))` if the slab is at capacity.
    pub fn try_schedule(&mut self, deadline: Instant, value: T) -> Result<TimerHandle<T>, Full<T>> {
        let deadline_ticks = self.instant_to_ticks(deadline);
        let entry = WheelEntry::new(deadline_ticks, value, 2);
        match self.slab.try_alloc(entry) {
            Ok(slot) => {
                let ptr = slot.as_ptr();
                self.insert_entry(ptr, deadline_ticks);
                self.len += 1;
                Ok(TimerHandle::new(ptr))
            }
            Err(full) => {
                // Extract the user's T from the WheelEntry wrapper
                // SAFETY: we just constructed this entry, take_value is valid
                let wheel_entry = full.into_inner();
                let value = unsafe { wheel_entry.take_value() }
                    .expect("entry was just constructed with Some(value)");
                Err(Full(value))
            }
        }
    }

    /// Attempts to schedule a fire-and-forget timer.
    ///
    /// Returns `Err(Full(value))` if the slab is at capacity.
    pub fn try_schedule_forget(&mut self, deadline: Instant, value: T) -> Result<(), Full<T>> {
        let deadline_ticks = self.instant_to_ticks(deadline);
        let entry = WheelEntry::new(deadline_ticks, value, 1);
        match self.slab.try_alloc(entry) {
            Ok(slot) => {
                let ptr = slot.as_ptr();
                self.insert_entry(ptr, deadline_ticks);
                self.len += 1;
                Ok(())
            }
            Err(full) => {
                let wheel_entry = full.into_inner();
                let value = unsafe { wheel_entry.take_value() }
                    .expect("entry was just constructed with Some(value)");
                Err(Full(value))
            }
        }
    }
}

// =============================================================================
// Cancel / Free / Poll / Query — generic over any store
// =============================================================================

impl<T: 'static, S: SlabStore<Item = WheelEntry<T>>> TimerWheel<T, S> {
    /// Cancels a timer and returns its value.
    ///
    /// - If the timer is still active: unlinks from the wheel, extracts value,
    ///   frees the slab entry. Returns `Some(T)`.
    /// - If the timer already fired (zombie handle): frees the slab entry.
    ///   Returns `None`.
    ///
    /// Consumes the handle (no Drop runs).
    pub fn cancel(&mut self, handle: TimerHandle<T>) -> Option<T> {
        let ptr = handle.ptr;
        // Consume handle without running Drop
        mem::forget(handle);

        // SAFETY: handle guarantees ptr is valid and allocated from our slab.
        let entry = unsafe { entry_ref(ptr) };
        let refs = entry.refs();

        if refs == 2 {
            // Active timer with handle — unlink, extract, free
            let value = unsafe { entry.take_value() };
            self.remove_entry(ptr);
            self.len -= 1;
            // SAFETY: ptr was allocated from our slab, entry is now spent
            unsafe { self.slab.free_ptr(ptr) };
            value
        } else {
            // refs == 1 means the wheel already fired this (zombie).
            // The fire path decremented 2→1 and left the entry for us to free.
            debug_assert_eq!(refs, 1, "unexpected refcount {refs} in cancel");
            // SAFETY: ptr was allocated from our slab
            unsafe { self.slab.free_ptr(ptr) };
            None
        }
    }

    /// Releases a timer handle without cancelling.
    ///
    /// - If the timer is still active: converts to fire-and-forget (refs 2→1).
    ///   Timer stays in the wheel and will fire normally during poll.
    /// - If the timer already fired (zombie): frees the slab entry (refs 1→0).
    ///
    /// Consumes the handle (no Drop runs).
    pub fn free(&mut self, handle: TimerHandle<T>) {
        let ptr = handle.ptr;
        mem::forget(handle);

        // SAFETY: handle guarantees ptr is valid
        let entry = unsafe { entry_ref(ptr) };
        let new_refs = entry.dec_refs();

        if new_refs == 0 {
            // Was a zombie (fired already, refs was 1) — free the entry
            // SAFETY: ptr was allocated from our slab
            unsafe { self.slab.free_ptr(ptr) };
        }
        // new_refs == 1: timer is now fire-and-forget, stays in wheel
    }

    /// Fires all expired timers, collecting their values into `buf`.
    ///
    /// Returns the number of timers fired.
    pub fn poll(&mut self, now: Instant, buf: &mut Vec<T>) -> usize {
        self.poll_with_limit(now, usize::MAX, buf)
    }

    /// Fires expired timers up to `limit`, collecting values into `buf`.
    ///
    /// Resumable: if the limit is hit, the next call continues where this one
    /// left off (as long as `now` hasn't changed).
    ///
    /// Returns the number of timers fired in this call.
    pub fn poll_with_limit(&mut self, now: Instant, limit: usize, buf: &mut Vec<T>) -> usize {
        let now_ticks = self.instant_to_ticks(now);
        self.current_ticks = now_ticks;

        // If now changed since the bookmark, reset
        if self.bookmark.ticks != now_ticks {
            self.bookmark.reset();
            self.bookmark.ticks = now_ticks;
        }

        let mut fired = 0;
        let num_levels = self.num_levels;

        let mut lvl_idx = self.bookmark.level;
        while lvl_idx < num_levels && fired < limit {
            fired += self.poll_level(lvl_idx, now_ticks, limit - fired, buf);

            if fired >= limit && lvl_idx < num_levels {
                // Bookmark was updated inside poll_level
                return fired;
            }

            // Level fully drained — advance to next
            self.bookmark.slot = std::ptr::null_mut();
            self.bookmark.entry = null_entry();
            lvl_idx += 1;
            self.bookmark.level = lvl_idx;
        }

        // All levels fully polled — reset bookmark
        self.bookmark.reset();
        fired
    }

    /// Returns the `Instant` of the next timer that will fire, or `None` if empty.
    ///
    /// Walks only active (non-empty) slots. O(active_slots) in the worst case,
    /// but typically very fast because most slots are empty.
    pub fn next_deadline(&self) -> Option<Instant> {
        let mut min_ticks: Option<u64> = None;

        for level in &self.levels {
            let mut slot_ptr = level.active_head();
            while !slot_ptr.is_null() {
                // SAFETY: slot_ptr is in the active list, therefore valid
                let slot = unsafe { &*slot_ptr };
                let mut entry_ptr = slot.entry_head();

                while !entry_ptr.is_null() {
                    // SAFETY: entry_ptr is in this slot's DLL, therefore valid
                    let entry = unsafe { entry_ref(entry_ptr) };
                    let dt = entry.deadline_ticks();

                    min_ticks = Some(min_ticks.map_or(dt, |current| current.min(dt)));

                    entry_ptr = entry.next();
                }

                slot_ptr = slot.active_next();
            }
        }

        min_ticks.map(|t| self.ticks_to_instant(t))
    }

    /// Returns the number of timers currently in the wheel.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the wheel contains no timers.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    // =========================================================================
    // Internal: tick conversion
    // =========================================================================

    #[inline]
    fn instant_to_ticks(&self, instant: Instant) -> u64 {
        // Saturate at 0 for instants before epoch
        let dur = instant.saturating_duration_since(self.epoch);
        dur.as_nanos() as u64 / self.tick_ns
    }

    #[inline]
    fn ticks_to_instant(&self, ticks: u64) -> Instant {
        self.epoch + Duration::from_nanos(ticks * self.tick_ns)
    }

    // =========================================================================
    // Internal: level selection
    // =========================================================================

    /// Selects the appropriate level for a deadline.
    ///
    /// Walks levels from finest to coarsest, picking the first level whose
    /// range can represent the delta. Clamps to the highest level if the
    /// deadline exceeds the wheel's total range.
    #[inline]
    fn select_level(&self, deadline_ticks: u64) -> usize {
        let delta = deadline_ticks.saturating_sub(self.current_ticks);

        for (i, level) in self.levels.iter().enumerate() {
            if delta < level.range() {
                return i;
            }
        }

        // Beyond max range — clamp to highest level
        self.num_levels - 1
    }

    // =========================================================================
    // Internal: entry insertion into a level's slot
    // =========================================================================

    /// Inserts an entry into the appropriate level and slot.
    ///
    /// Records the level and slot index on the entry so `remove_entry` can
    /// find it without recomputing (which would be unsound after time advances).
    #[inline]
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn insert_entry(&mut self, entry_ptr: EntryPtr<T>, deadline_ticks: u64) {
        let lvl_idx = self.select_level(deadline_ticks);
        let level = &self.levels[lvl_idx];
        let slot_idx = level.slot_index(deadline_ticks);
        let slot_ptr = level.slot_ptr(slot_idx);
        let slot = level.slot(slot_idx);

        // Record location on the entry for O(1) lookup at cancel time.
        // SAFETY: entry_ptr is valid (just allocated)
        let entry = unsafe { entry_ref(entry_ptr) };
        entry.set_location(lvl_idx as u8, slot_idx as u16);

        let was_empty = slot.is_empty();

        // SAFETY: entry_ptr is valid (just allocated), not in any DLL yet
        unsafe { slot.push_entry(entry_ptr) };

        if was_empty {
            // SAFETY: slot_ptr is within this level, not already in active list
            unsafe { level.link_active(slot_ptr) };
        }
    }

    /// Removes an entry from its level's slot DLL.
    ///
    /// Reads the stored level and slot index from the entry (set at insertion
    /// time). Does NOT recompute from delta — that would be unsound after
    /// `current_ticks` advances.
    #[inline]
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn remove_entry(&mut self, entry_ptr: EntryPtr<T>) {
        // SAFETY: entry_ptr is valid (caller guarantee)
        let entry = unsafe { entry_ref(entry_ptr) };

        let lvl_idx = entry.level() as usize;
        let slot_idx = entry.slot_idx() as usize;

        let level = &self.levels[lvl_idx];
        let slot_ptr = level.slot_ptr(slot_idx);
        let slot = level.slot(slot_idx);

        // SAFETY: entry_ptr is in this slot's DLL (invariant from insert_entry)
        unsafe { slot.remove_entry(entry_ptr) };

        if slot.is_empty() {
            // SAFETY: slot_ptr was in the active list (had entries)
            unsafe { level.unlink_active(slot_ptr) };
        }
    }

    // =========================================================================
    // Internal: fire an entry
    // =========================================================================

    /// Fires a single entry: extracts value, decrements refcount, possibly frees.
    ///
    /// Returns `Some(T)` if the value was still present (not already cancelled).
    #[inline]
    fn fire_entry(&mut self, entry_ptr: EntryPtr<T>) -> Option<T> {
        // SAFETY: entry_ptr is valid (we're walking the DLL)
        let entry = unsafe { entry_ref(entry_ptr) };

        // Extract value
        // SAFETY: single-threaded
        let value = unsafe { entry.take_value() };

        let new_refs = entry.dec_refs();
        if new_refs == 0 {
            // Fire-and-forget (was refs=1) — free the slab entry immediately
            // SAFETY: entry_ptr was allocated from our slab
            unsafe { self.slab.free_ptr(entry_ptr) };
        }
        // new_refs == 1: handle exists (was refs=2), entry becomes zombie.
        // Handle holder will free via cancel() or free().

        self.len -= 1;
        value
    }

    // =========================================================================
    // Internal: poll a single level
    // =========================================================================

    /// Polls a single level for expired entries up to `limit`.
    ///
    /// Updates the bookmark for resumability.
    fn poll_level(
        &mut self,
        lvl_idx: usize,
        now_ticks: u64,
        limit: usize,
        buf: &mut Vec<T>,
    ) -> usize {
        let mut fired = 0;

        let level = &self.levels[lvl_idx];

        // Resume from bookmark or start from active_head
        let mut slot_ptr = if self.bookmark.level == lvl_idx && !self.bookmark.slot.is_null() {
            self.bookmark.slot
        } else {
            level.active_head()
        };

        while !slot_ptr.is_null() && fired < limit {
            // SAFETY: slot_ptr is in the active list
            let slot = unsafe { &*slot_ptr };
            let next_slot = slot.active_next();

            // Resume entry position or start from head
            let mut entry_ptr = if self.bookmark.level == lvl_idx
                && self.bookmark.slot == slot_ptr
                && !self.bookmark.entry.is_null()
            {
                self.bookmark.entry
            } else {
                slot.entry_head()
            };

            while !entry_ptr.is_null() && fired < limit {
                // SAFETY: entry_ptr is in this slot's DLL
                let entry = unsafe { entry_ref(entry_ptr) };
                let next_entry = entry.next();

                if entry.deadline_ticks() <= now_ticks {
                    // Expired — unlink from DLL then fire
                    // SAFETY: entry_ptr is in this slot's DLL
                    unsafe { slot.remove_entry(entry_ptr) };

                    if let Some(value) = self.fire_entry(entry_ptr) {
                        buf.push(value);
                    }
                    fired += 1;
                }

                entry_ptr = next_entry;
            }

            // If we exhausted limit mid-slot, bookmark for resumption
            if fired >= limit && entry_ptr != null_entry() {
                self.bookmark.level = lvl_idx;
                self.bookmark.slot = slot_ptr;
                self.bookmark.entry = entry_ptr;

                // Check if slot became empty after removals
                if slot.is_empty() {
                    let level = &self.levels[lvl_idx];
                    // SAFETY: slot_ptr was in the active list
                    unsafe { level.unlink_active(slot_ptr) };
                }
                return fired;
            }

            // Check if slot became empty after removals
            if slot.is_empty() {
                let level = &self.levels[lvl_idx];
                // SAFETY: slot_ptr was in the active list
                unsafe { level.unlink_active(slot_ptr) };
            }

            slot_ptr = next_slot;
        }

        // Level fully polled (or empty)
        self.bookmark.slot = std::ptr::null_mut();
        self.bookmark.entry = null_entry();
        fired
    }
}

// =============================================================================
// Drop
// =============================================================================

impl<T: 'static, S: SlabStore<Item = WheelEntry<T>>> Drop for TimerWheel<T, S> {
    fn drop(&mut self) {
        // Walk all active-slot lists and free every entry.
        // Active entries have Some(value) — slab.free calls drop_in_place which
        // drops the WheelEntry, and the Option<T> inside it drops the T.
        for level in &self.levels {
            let mut slot_ptr = level.active_head();
            while !slot_ptr.is_null() {
                // SAFETY: slot_ptr is in the active list
                let slot = unsafe { &*slot_ptr };
                let next_slot = slot.active_next();

                let mut entry_ptr = slot.entry_head();
                while !entry_ptr.is_null() {
                    // SAFETY: entry_ptr is in this slot's DLL
                    let entry = unsafe { entry_ref(entry_ptr) };
                    let next_entry = entry.next();

                    // SAFETY: entry_ptr was allocated from our slab
                    unsafe { self.slab.free(nexus_slab::Slot::from_ptr(entry_ptr)) };

                    entry_ptr = next_entry;
                }

                slot_ptr = next_slot;
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
    use std::time::{Duration, Instant};

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    // -------------------------------------------------------------------------
    // Construction
    // -------------------------------------------------------------------------

    #[test]
    fn default_config() {
        let now = Instant::now();
        let wheel: Wheel<u64> = Wheel::unbounded(1024, now);
        assert!(wheel.is_empty());
        assert_eq!(wheel.len(), 0);
    }

    #[test]
    fn bounded_construction() {
        let now = Instant::now();
        let wheel: BoundedWheel<u64> = BoundedWheel::bounded(128, now);
        assert!(wheel.is_empty());
    }

    #[test]
    #[should_panic(expected = "slots_per_level must be a power of 2")]
    fn invalid_config_non_power_of_two() {
        let now = Instant::now();
        WheelBuilder::default()
            .slots_per_level(65)
            .unbounded(1024)
            .build::<u64>(now);
    }

    // -------------------------------------------------------------------------
    // Schedule + Cancel
    // -------------------------------------------------------------------------

    #[test]
    fn schedule_and_cancel() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        let h = wheel.schedule(now + ms(50), 42);
        assert_eq!(wheel.len(), 1);

        let val = wheel.cancel(h);
        assert_eq!(val, Some(42));
        assert_eq!(wheel.len(), 0);
    }

    #[test]
    fn schedule_forget_fires() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        wheel.schedule_forget(now + ms(10), 99);
        assert_eq!(wheel.len(), 1);

        let mut buf = Vec::new();
        let fired = wheel.poll(now + ms(20), &mut buf);
        assert_eq!(fired, 1);
        assert_eq!(buf, vec![99]);
        assert_eq!(wheel.len(), 0);
    }

    #[test]
    fn cancel_after_fire_returns_none() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        let h = wheel.schedule(now + ms(10), 42);

        let mut buf = Vec::new();
        wheel.poll(now + ms(20), &mut buf);
        assert_eq!(buf, vec![42]);

        // Handle is now a zombie
        let val = wheel.cancel(h);
        assert_eq!(val, None);
    }

    #[test]
    fn free_active_timer_becomes_fire_and_forget() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        let h = wheel.schedule(now + ms(10), 42);
        wheel.free(h); // releases handle, timer stays
        assert_eq!(wheel.len(), 1);

        let mut buf = Vec::new();
        wheel.poll(now + ms(20), &mut buf);
        assert_eq!(buf, vec![42]);
        assert_eq!(wheel.len(), 0);
    }

    #[test]
    fn free_zombie_handle() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        let h = wheel.schedule(now + ms(10), 42);

        let mut buf = Vec::new();
        wheel.poll(now + ms(20), &mut buf);

        // Handle is zombie, free should clean up
        wheel.free(h);
    }

    // -------------------------------------------------------------------------
    // Bounded wheel
    // -------------------------------------------------------------------------

    #[test]
    fn bounded_full() {
        let now = Instant::now();
        let mut wheel: BoundedWheel<u64> = BoundedWheel::bounded(2, now);

        let h1 = wheel.try_schedule(now + ms(10), 1).unwrap();
        let h2 = wheel.try_schedule(now + ms(20), 2).unwrap();

        let err = wheel.try_schedule(now + ms(30), 3);
        assert!(err.is_err());
        let recovered = err.unwrap_err().into_inner();
        assert_eq!(recovered, 3);

        // Cancel one, should have room
        wheel.cancel(h1);
        let h3 = wheel.try_schedule(now + ms(30), 3).unwrap();

        // Clean up handles
        wheel.free(h2);
        wheel.free(h3);
    }

    #[test]
    fn bounded_schedule_forget_full() {
        let now = Instant::now();
        let mut wheel: BoundedWheel<u64> = BoundedWheel::bounded(1, now);

        wheel.try_schedule_forget(now + ms(10), 1).unwrap();
        let err = wheel.try_schedule_forget(now + ms(20), 2);
        assert!(err.is_err());
    }

    // -------------------------------------------------------------------------
    // Poll
    // -------------------------------------------------------------------------

    #[test]
    fn poll_respects_deadline() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        wheel.schedule_forget(now + ms(10), 1);
        wheel.schedule_forget(now + ms(50), 2);
        wheel.schedule_forget(now + ms(100), 3);

        let mut buf = Vec::new();

        // At 20ms: only timer 1 should fire
        let fired = wheel.poll(now + ms(20), &mut buf);
        assert_eq!(fired, 1);
        assert_eq!(buf, vec![1]);
        assert_eq!(wheel.len(), 2);

        // At 60ms: timer 2 fires
        buf.clear();
        let fired = wheel.poll(now + ms(60), &mut buf);
        assert_eq!(fired, 1);
        assert_eq!(buf, vec![2]);

        // At 200ms: timer 3 fires
        buf.clear();
        let fired = wheel.poll(now + ms(200), &mut buf);
        assert_eq!(fired, 1);
        assert_eq!(buf, vec![3]);

        assert!(wheel.is_empty());
    }

    #[test]
    fn poll_with_limit() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        for i in 0..10 {
            wheel.schedule_forget(now + ms(1), i);
        }

        let mut buf = Vec::new();

        // Fire 3 at a time
        let fired = wheel.poll_with_limit(now + ms(5), 3, &mut buf);
        assert_eq!(fired, 3);
        assert_eq!(wheel.len(), 7);

        let fired = wheel.poll_with_limit(now + ms(5), 3, &mut buf);
        assert_eq!(fired, 3);
        assert_eq!(wheel.len(), 4);

        // Fire remaining
        let fired = wheel.poll(now + ms(5), &mut buf);
        assert_eq!(fired, 4);
        assert!(wheel.is_empty());
        assert_eq!(buf.len(), 10);
    }

    // -------------------------------------------------------------------------
    // Multi-level
    // -------------------------------------------------------------------------

    #[test]
    fn timers_across_levels() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // Level 0: 0-63ms
        wheel.schedule_forget(now + ms(5), 0);
        // Level 1: 64-511ms
        wheel.schedule_forget(now + ms(200), 1);
        // Level 2: 512-4095ms
        wheel.schedule_forget(now + ms(1000), 2);

        let mut buf = Vec::new();

        wheel.poll(now + ms(10), &mut buf);
        assert_eq!(buf, vec![0]);

        buf.clear();
        wheel.poll(now + ms(250), &mut buf);
        assert_eq!(buf, vec![1]);

        buf.clear();
        wheel.poll(now + ms(1500), &mut buf);
        assert_eq!(buf, vec![2]);

        assert!(wheel.is_empty());
    }

    // -------------------------------------------------------------------------
    // next_deadline
    // -------------------------------------------------------------------------

    #[test]
    fn next_deadline_empty() {
        let now = Instant::now();
        let wheel: Wheel<u64> = Wheel::unbounded(1024, now);
        assert!(wheel.next_deadline().is_none());
    }

    #[test]
    fn next_deadline_returns_earliest() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        wheel.schedule_forget(now + ms(100), 1);
        wheel.schedule_forget(now + ms(50), 2);
        wheel.schedule_forget(now + ms(200), 3);

        let next = wheel.next_deadline().unwrap();
        // Should be close to now + 50ms (within tick granularity)
        let delta = next.duration_since(now);
        assert!(delta >= ms(49) && delta <= ms(51));
    }

    // -------------------------------------------------------------------------
    // Deadline in the past
    // -------------------------------------------------------------------------

    #[test]
    fn deadline_in_the_past_fires_immediately() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // Schedule at epoch (which is "now" at construction)
        wheel.schedule_forget(now, 42);

        let mut buf = Vec::new();
        let fired = wheel.poll(now + ms(1), &mut buf);
        assert_eq!(fired, 1);
        assert_eq!(buf, vec![42]);
    }

    // -------------------------------------------------------------------------
    // Deadline beyond max range — clamped
    // -------------------------------------------------------------------------

    #[test]
    fn deadline_beyond_max_range_clamped() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // Way in the future — should clamp to highest level
        let h = wheel.schedule(now + Duration::from_secs(100_000), 99);
        assert_eq!(wheel.len(), 1);

        // Won't fire at any reasonable time but will fire when enough ticks pass
        let mut buf = Vec::new();
        wheel.poll(now + Duration::from_secs(100_001), &mut buf);
        assert_eq!(buf, vec![99]);

        // Note: handle was already consumed by the poll (fire-and-forget path won't
        // apply since refs=2). Actually the handle still exists. Let's clean up.
        // The timer already fired, so cancel returns None.
        // Actually buf got the value, which means it fired. But handle still needs cleanup.
        // We already pushed the value so we need to handle the zombie.
        // Wait — we used schedule (refs=2), poll fired it (refs 2→1 zombie), handle `h` exists.
        // Actually we consumed it with the poll — no we didn't, we still have `h`.

        // h is a zombie handle now
        let val = wheel.cancel(h);
        assert_eq!(val, None);
    }

    // -------------------------------------------------------------------------
    // Drop
    // -------------------------------------------------------------------------

    #[test]
    fn drop_cleans_up_active_entries() {
        let now = Instant::now();
        let mut wheel: Wheel<String> = Wheel::unbounded(1024, now);

        for i in 0..100 {
            wheel.schedule_forget(now + ms(i * 10), format!("timer-{i}"));
        }

        assert_eq!(wheel.len(), 100);
        // Drop should free all entries without leaking
        drop(wheel);
    }

    #[test]
    fn drop_with_outstanding_handles() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // Schedule but DON'T cancel — just free the handles
        let h1 = wheel.schedule(now + ms(10), 1);
        let h2 = wheel.schedule(now + ms(20), 2);

        // Free the handles (convert to fire-and-forget) so they don't debug_assert
        wheel.free(h1);
        wheel.free(h2);

        // Drop the wheel — should clean up the entries
        drop(wheel);
    }

    // -------------------------------------------------------------------------
    // Level selection
    // -------------------------------------------------------------------------

    #[test]
    fn level_selection_boundaries() {
        let now = Instant::now();
        let wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // Level 0: delta < 64
        assert_eq!(wheel.select_level(0), 0);
        assert_eq!(wheel.select_level(63), 0);

        // Level 1: 64 <= delta < 512
        assert_eq!(wheel.select_level(64), 1);
        assert_eq!(wheel.select_level(511), 1);

        // Level 2: 512 <= delta < 4096
        assert_eq!(wheel.select_level(512), 2);
    }

    // -------------------------------------------------------------------------
    // Bug fix validation: cancel after time advance
    // -------------------------------------------------------------------------

    #[test]
    fn cancel_after_time_advance() {
        // The critical bug: schedule at T+500ms (level 2, delta=500 ticks),
        // poll at T+400ms (no fire, but current_ticks advances to 400),
        // cancel at T+400ms. Old code would recompute delta = 500-400 = 100
        // → level 1. But the entry is in level 2. Stored location fixes this.
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        let h = wheel.schedule(now + ms(500), 42);
        assert_eq!(wheel.len(), 1);

        // Advance time — timer doesn't fire (deadline is 500ms)
        let mut buf = Vec::new();
        let fired = wheel.poll(now + ms(400), &mut buf);
        assert_eq!(fired, 0);
        assert!(buf.is_empty());

        // Cancel after time advance — must find the entry in the correct slot
        let val = wheel.cancel(h);
        assert_eq!(val, Some(42));
        assert_eq!(wheel.len(), 0);
    }

    // -------------------------------------------------------------------------
    // Same-slot entries
    // -------------------------------------------------------------------------

    #[test]
    fn multiple_entries_same_slot() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // All 5 timers at the same deadline → same slot
        let mut handles = Vec::new();
        for i in 0..5 {
            handles.push(wheel.schedule(now + ms(10), i));
        }
        assert_eq!(wheel.len(), 5);

        // Cancel the middle ones
        let v2 = wheel.cancel(handles.remove(2));
        assert_eq!(v2, Some(2));
        let v0 = wheel.cancel(handles.remove(0));
        assert_eq!(v0, Some(0));
        assert_eq!(wheel.len(), 3);

        // Poll — remaining 3 should fire
        let mut buf = Vec::new();
        let fired = wheel.poll(now + ms(20), &mut buf);
        assert_eq!(fired, 3);

        // Clean up zombie handles
        for h in handles {
            let val = wheel.cancel(h);
            assert_eq!(val, None); // already fired
        }
    }

    // -------------------------------------------------------------------------
    // Level boundary
    // -------------------------------------------------------------------------

    #[test]
    fn entry_at_level_boundary() {
        // Default config: level 0 range = 64 ticks (64ms).
        // A deadline at exactly tick 64 should go to level 1, not level 0.
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        let h = wheel.schedule(now + ms(64), 99);
        assert_eq!(wheel.len(), 1);

        // Should NOT fire at 63ms
        let mut buf = Vec::new();
        let fired = wheel.poll(now + ms(63), &mut buf);
        assert_eq!(fired, 0);

        // Should fire at 64ms
        let fired = wheel.poll(now + ms(65), &mut buf);
        assert_eq!(fired, 1);
        assert_eq!(buf, vec![99]);

        // Clean up zombie handle
        wheel.cancel(h);
    }

    // -------------------------------------------------------------------------
    // Bookmark/resumption with mixed expiry
    // -------------------------------------------------------------------------

    #[test]
    fn poll_with_limit_mixed_expiry() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // 3 expired at poll time, 2 not
        wheel.schedule_forget(now + ms(5), 1);
        wheel.schedule_forget(now + ms(5), 2);
        wheel.schedule_forget(now + ms(5), 3);
        wheel.schedule_forget(now + ms(500), 4); // not expired
        wheel.schedule_forget(now + ms(500), 5); // not expired
        assert_eq!(wheel.len(), 5);

        let mut buf = Vec::new();

        // Fire 2 of the 3 expired
        let fired = wheel.poll_with_limit(now + ms(10), 2, &mut buf);
        assert_eq!(fired, 2);
        assert_eq!(wheel.len(), 3);

        // Fire remaining expired (1 more)
        let fired = wheel.poll_with_limit(now + ms(10), 5, &mut buf);
        assert_eq!(fired, 1);
        assert_eq!(wheel.len(), 2);

        // The 2 unexpired should still be there
        assert_eq!(buf.len(), 3);
    }

    // -------------------------------------------------------------------------
    // Re-add after drain
    // -------------------------------------------------------------------------

    #[test]
    fn reuse_after_full_drain() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // Round 1: schedule and drain
        for i in 0..10 {
            wheel.schedule_forget(now + ms(1), i);
        }
        let mut buf = Vec::new();
        wheel.poll(now + ms(5), &mut buf);
        assert_eq!(buf.len(), 10);
        assert!(wheel.is_empty());

        // Round 2: schedule and drain again — wheel must work normally
        buf.clear();
        for i in 10..20 {
            wheel.schedule_forget(now + ms(100), i);
        }
        assert_eq!(wheel.len(), 10);

        wheel.poll(now + ms(200), &mut buf);
        assert_eq!(buf.len(), 10);
        assert!(wheel.is_empty());
    }

    // -------------------------------------------------------------------------
    // All levels active simultaneously
    // -------------------------------------------------------------------------

    #[test]
    fn all_levels_active() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        // Schedule one timer per level with increasing distances.
        // Level 0: <64ms, Level 1: 64-511ms, Level 2: 512-4095ms, etc.
        let distances = [10, 100, 1000, 5000, 40_000, 300_000, 3_000_000];
        let mut handles: Vec<TimerHandle<u64>> = Vec::new();
        for (i, &d) in distances.iter().enumerate() {
            handles.push(wheel.schedule(now + ms(d), i as u64));
        }
        assert_eq!(wheel.len(), 7);

        // Cancel in a shuffled order: 4, 1, 6, 0, 3, 5, 2
        let order = [4, 1, 6, 0, 3, 5, 2];
        // Take ownership by swapping with dummies — actually we need to
        // cancel by index. Let's use Option to track.
        let mut opt_handles: Vec<Option<TimerHandle<u64>>> =
            handles.into_iter().map(Some).collect();

        for &idx in &order {
            let h = opt_handles[idx].take().unwrap();
            let val = wheel.cancel(h);
            assert_eq!(val, Some(idx as u64));
        }
        assert!(wheel.is_empty());
    }

    // -------------------------------------------------------------------------
    // Poll values match
    // -------------------------------------------------------------------------

    #[test]
    fn poll_values_match() {
        let now = Instant::now();
        let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

        let expected: Vec<u64> = (100..110).collect();
        for &v in &expected {
            wheel.schedule_forget(now + ms(5), v);
        }

        let mut buf = Vec::new();
        wheel.poll(now + ms(10), &mut buf);

        buf.sort();
        assert_eq!(buf, expected);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;
    use std::mem;
    use std::time::{Duration, Instant};

    /// Operation in a schedule/cancel interleaving.
    #[derive(Debug, Clone)]
    enum Op {
        /// Schedule a timer at `deadline_ms` milliseconds from epoch.
        Schedule { deadline_ms: u64 },
        /// Cancel the timer at the given index (modulo outstanding handles).
        Cancel { idx: usize },
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            // Schedule with deadlines from 1ms to 10_000ms
            (1u64..10_000).prop_map(|deadline_ms| Op::Schedule { deadline_ms }),
            // Cancel at random index
            any::<usize>().prop_map(|idx| Op::Cancel { idx }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(500))]

        /// Fuzz schedule/cancel interleaving.
        ///
        /// Random sequence of schedule and cancel operations. Invariants:
        /// - `len` always matches outstanding active timers
        /// - cancel on active handle returns `Some`
        /// - poll collects all un-cancelled values
        #[test]
        fn fuzz_schedule_cancel_interleaving(ops in proptest::collection::vec(op_strategy(), 1..200)) {
            let now = Instant::now();
            let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

            let mut handles: Vec<TimerHandle<u64>> = Vec::new();
            let mut active_values: HashSet<u64> = HashSet::new();
            let mut next_id: u64 = 0;

            for op in &ops {
                match op {
                    Op::Schedule { deadline_ms } => {
                        let id = next_id;
                        next_id += 1;
                        let h = wheel.schedule(now + Duration::from_millis(*deadline_ms), id);
                        handles.push(h);
                        active_values.insert(id);
                    }
                    Op::Cancel { idx } => {
                        if !handles.is_empty() {
                            let i = idx % handles.len();
                            let h = handles.swap_remove(i);
                            let val = wheel.cancel(h);
                            // Value should be Some (all handles are for active timers)
                            let v = val.unwrap();
                            assert!(active_values.remove(&v));
                        }
                    }
                }
                // len must match active values
                prop_assert_eq!(wheel.len(), active_values.len());
            }

            // Poll everything — should collect exactly the remaining active values
            let mut buf = Vec::new();
            // Use a far-future time to fire everything
            wheel.poll(now + Duration::from_secs(100_000), &mut buf);

            // Clean up zombie handles (poll fired them, handles still exist)
            for h in handles {
                mem::forget(h);
            }

            let fired_set: HashSet<u64> = buf.into_iter().collect();
            prop_assert_eq!(fired_set, active_values);
            prop_assert!(wheel.is_empty());
        }

        /// Fuzz poll timing.
        ///
        /// Schedule N timers with random deadlines. Poll at random increasing
        /// times. Assert every timer fires exactly once, fired deadlines are
        /// all <= poll time, unfired deadlines are all > poll time.
        #[test]
        fn fuzz_poll_timing(
            deadlines in proptest::collection::vec(1u64..5000, 1..100),
            poll_times in proptest::collection::vec(1u64..10_000, 1..20),
        ) {
            let now = Instant::now();
            let mut wheel: Wheel<u64> = Wheel::unbounded(1024, now);

            // Schedule all timers (fire-and-forget)
            for (i, &d) in deadlines.iter().enumerate() {
                wheel.schedule_forget(now + Duration::from_millis(d), i as u64);
            }

            // Sort poll times to be monotonically increasing
            let mut sorted_times: Vec<u64> = poll_times;
            sorted_times.sort();
            sorted_times.dedup();

            let mut all_fired: Vec<u64> = Vec::new();

            for &t in &sorted_times {
                let mut buf = Vec::new();
                wheel.poll(now + Duration::from_millis(t), &mut buf);

                // Every fired entry should have deadline_ms <= t
                for &id in &buf {
                    let deadline_ms = deadlines[id as usize];
                    prop_assert!(deadline_ms <= t,
                        "Timer {} with deadline {}ms fired at {}ms", id, deadline_ms, t);
                }

                all_fired.extend(buf);
            }

            // Fire everything remaining
            let mut final_buf = Vec::new();
            wheel.poll(now + Duration::from_secs(100_000), &mut final_buf);
            all_fired.extend(final_buf);

            // Every timer should have fired exactly once
            all_fired.sort();
            let expected: Vec<u64> = (0..deadlines.len() as u64).collect();
            prop_assert_eq!(all_fired, expected, "Not all timers fired exactly once");
            prop_assert!(wheel.is_empty());
        }
    }
}
