//! Timer wheel level — one tier in the hierarchical wheel.
//!
//! Each level has `slots_per_level` slots (default 64). Entries are placed
//! into slots based on their deadline. Non-empty slots are linked into an
//! intrusive doubly-linked "active-slot" list so poll/next_deadline only
//! visit slots that actually contain entries.
//!
//! Two separate DLLs operate here:
//! 1. **Entry DLL** — per-slot list of entries (WheelEntry prev/next).
//! 2. **Active-slot DLL** — per-level list of non-empty slots.

use std::cell::Cell;
use std::ptr;

use crate::entry::{EntryPtr, entry_ref, null_entry};

// =============================================================================
// WheelSlot — one slot position within a level
// =============================================================================

/// A single slot in a timer wheel level.
///
/// Contains the head/tail of the entry DLL and links for the active-slot DLL.
pub(crate) struct WheelSlot<T> {
    /// Head of the entry DLL (first entry in this slot, or null).
    entry_head: Cell<EntryPtr<T>>,
    /// Tail of the entry DLL (last entry, for O(1) append).
    entry_tail: Cell<EntryPtr<T>>,
    /// Previous non-empty slot in the level's active-slot list (or null).
    active_prev: Cell<*mut WheelSlot<T>>,
    /// Next non-empty slot in the level's active-slot list (or null).
    active_next: Cell<*mut WheelSlot<T>>,
    /// This slot's index within the level (for diagnostics).
    #[allow(dead_code)]
    slot_index: u32,
}

impl<T: 'static> WheelSlot<T> {
    fn new(slot_index: u32) -> Self {
        WheelSlot {
            entry_head: Cell::new(null_entry()),
            entry_tail: Cell::new(null_entry()),
            active_prev: Cell::new(ptr::null_mut()),
            active_next: Cell::new(ptr::null_mut()),
            slot_index,
        }
    }

    /// Returns true if this slot has no entries.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.entry_head.get().is_null()
    }

    /// Returns this slot's index within the level.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn slot_index(&self) -> u32 {
        self.slot_index
    }

    // =========================================================================
    // Entry DLL operations
    // =========================================================================

    /// Appends an entry to the tail of this slot's DLL.
    ///
    /// # Safety
    ///
    /// `entry_ptr` must be a valid, non-null pointer to an occupied slab slot
    /// containing a `WheelEntry<T>`. The entry must not already be in any DLL.
    #[inline]
    pub(crate) unsafe fn push_entry(&self, entry_ptr: EntryPtr<T>) {
        // SAFETY: caller guarantees entry_ptr is valid and occupied
        let entry = unsafe { entry_ref(entry_ptr) };
        entry.set_prev(null_entry());
        entry.set_next(null_entry());

        let tail = self.entry_tail.get();
        if tail.is_null() {
            // Empty list — entry becomes both head and tail
            self.entry_head.set(entry_ptr);
        } else {
            // Append after current tail
            // SAFETY: tail is valid (was set from a previous push)
            let tail_entry = unsafe { entry_ref(tail) };
            tail_entry.set_next(entry_ptr);
            entry.set_prev(tail);
        }
        self.entry_tail.set(entry_ptr);
    }

    /// Removes an entry from this slot's DLL.
    ///
    /// # Safety
    ///
    /// `entry_ptr` must be in THIS slot's DLL. Caller must ensure the entry
    /// is currently linked (not already removed).
    #[inline]
    pub(crate) unsafe fn remove_entry(&self, entry_ptr: EntryPtr<T>) {
        // SAFETY: caller guarantees entry_ptr is valid and in this slot's DLL
        let entry = unsafe { entry_ref(entry_ptr) };
        let prev = entry.prev();
        let next = entry.next();

        if prev.is_null() {
            // Entry was the head
            self.entry_head.set(next);
        } else {
            // SAFETY: prev is valid (was set from a previous push/link)
            unsafe { entry_ref(prev) }.set_next(next);
        }

        if next.is_null() {
            // Entry was the tail
            self.entry_tail.set(prev);
        } else {
            // SAFETY: next is valid (was set from a previous push/link)
            unsafe { entry_ref(next) }.set_prev(prev);
        }

        // Clear links so a double-remove is detectable
        entry.set_prev(null_entry());
        entry.set_next(null_entry());
    }

    /// Returns the head entry pointer (may be null if slot is empty).
    #[inline]
    pub(crate) fn entry_head(&self) -> EntryPtr<T> {
        self.entry_head.get()
    }

    /// Returns the next non-empty slot in the active-slot list (or null).
    #[inline]
    pub(crate) fn active_next(&self) -> *mut WheelSlot<T> {
        self.active_next.get()
    }
}

// =============================================================================
// Level — one tier of the wheel
// =============================================================================

/// One level (tier) in the hierarchical timer wheel.
///
/// All levels have the same number of slots (`slots_per_level`). Higher levels
/// cover wider time ranges with coarser granularity.
pub(crate) struct Level<T> {
    /// Slot storage — `slots_per_level` slots, heap-allocated once at construction.
    slots: Box<[WheelSlot<T>]>,
    /// Head of the active-slot DLL (first non-empty slot, or null).
    active_head: Cell<*mut WheelSlot<T>>,
    /// Bit shift for this level: `level_index * clk_shift`.
    /// Level 0 shift = 0, level 1 shift = clk_shift, level 2 = 2*clk_shift, ...
    shift: u32,
    /// Bitmask: `slots_per_level - 1` (e.g. 63 for 64 slots).
    mask: usize,
    /// Number of ticks this level spans: `slots_per_level << shift`.
    range: u64,
}

impl<T: 'static> Level<T> {
    /// Creates a new level with the given parameters.
    pub(crate) fn new(slots_per_level: usize, level_index: usize, clk_shift: u32) -> Self {
        let shift = (level_index as u32) * clk_shift;

        let slots: Vec<WheelSlot<T>> = (0..slots_per_level)
            .map(|i| WheelSlot::new(i as u32))
            .collect();

        Level {
            slots: slots.into_boxed_slice(),
            active_head: Cell::new(ptr::null_mut()),
            shift,
            mask: slots_per_level - 1,
            range: (slots_per_level as u64) << shift,
        }
    }

    /// Returns the shift for this level.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn shift(&self) -> u32 {
        self.shift
    }

    /// Returns the mask for this level.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn mask(&self) -> usize {
        self.mask
    }

    /// Returns the range (in ticks) this level covers.
    #[inline]
    pub(crate) fn range(&self) -> u64 {
        self.range
    }

    /// Computes the slot index for a given deadline.
    #[inline]
    pub(crate) fn slot_index(&self, deadline_ticks: u64) -> usize {
        ((deadline_ticks >> self.shift) as usize) & self.mask
    }

    /// Returns a pointer to the slot at the given index.
    #[inline]
    pub(crate) fn slot_ptr(&self, index: usize) -> *mut WheelSlot<T> {
        debug_assert!(index < self.slots.len());
        // SAFETY: index is within bounds (checked in debug). We return a *mut
        // from a &self reference — sound because all mutation goes through Cell.
        self.slots.as_ptr().cast_mut().wrapping_add(index)
    }

    /// Returns a reference to the slot at the given index.
    #[inline]
    pub(crate) fn slot(&self, index: usize) -> &WheelSlot<T> {
        debug_assert!(index < self.slots.len());
        unsafe { &*self.slot_ptr(index) }
    }

    // =========================================================================
    // Active-slot DLL operations
    // =========================================================================

    /// Links a slot into the active-slot list (called when first entry added).
    ///
    /// # Safety
    ///
    /// `slot_ptr` must point to a slot within this level. The slot must not
    /// already be in the active list.
    #[inline]
    pub(crate) unsafe fn link_active(&self, slot_ptr: *mut WheelSlot<T>) {
        // SAFETY: slot_ptr is valid (caller guarantee)
        let slot = unsafe { &*slot_ptr };
        slot.active_prev.set(ptr::null_mut());

        let head = self.active_head.get();
        slot.active_next.set(head);

        if !head.is_null() {
            // SAFETY: head is valid (was previously linked)
            unsafe { &*head }.active_prev.set(slot_ptr);
        }
        self.active_head.set(slot_ptr);
    }

    /// Unlinks a slot from the active-slot list (called when last entry removed).
    ///
    /// # Safety
    ///
    /// `slot_ptr` must be currently in this level's active list.
    #[inline]
    pub(crate) unsafe fn unlink_active(&self, slot_ptr: *mut WheelSlot<T>) {
        // SAFETY: slot_ptr is valid (caller guarantee)
        let slot = unsafe { &*slot_ptr };
        let prev = slot.active_prev.get();
        let next = slot.active_next.get();

        if prev.is_null() {
            // Slot was the head
            self.active_head.set(next);
        } else {
            // SAFETY: prev is valid (was linked)
            unsafe { &*prev }.active_next.set(next);
        }

        if !next.is_null() {
            // SAFETY: next is valid (was linked)
            unsafe { &*next }.active_prev.set(prev);
        }

        slot.active_prev.set(ptr::null_mut());
        slot.active_next.set(ptr::null_mut());
    }

    /// Returns the head of the active-slot list (or null if no active slots).
    #[inline]
    pub(crate) fn active_head(&self) -> *mut WheelSlot<T> {
        self.active_head.get()
    }

    /// Returns the number of slots per level.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn num_slots(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::WheelEntry;
    use nexus_slab::unbounded;

    fn make_entry<T>(
        slab: &unbounded::Slab<WheelEntry<T>>,
        deadline: u64,
        value: T,
    ) -> EntryPtr<T> {
        let entry = WheelEntry::new(deadline, value, 1);
        let slot = slab.alloc(entry);
        slot.as_ptr()
    }

    #[test]
    fn slot_push_remove_single() {
        let slab = unbounded::Slab::<WheelEntry<u64>>::with_chunk_capacity(16);
        let ws = WheelSlot::<u64>::new(0);

        assert!(ws.is_empty());

        let e1 = make_entry(&slab, 10, 100);
        unsafe { ws.push_entry(e1) };
        assert!(!ws.is_empty());

        unsafe { ws.remove_entry(e1) };
        assert!(ws.is_empty());

        // SAFETY: e1 was allocated from slab
        unsafe { slab.free_ptr(e1) };
    }

    #[test]
    fn slot_push_remove_multiple() {
        let slab = unbounded::Slab::<WheelEntry<u64>>::with_chunk_capacity(16);
        let ws = WheelSlot::<u64>::new(0);

        let e1 = make_entry(&slab, 10, 1);
        let e2 = make_entry(&slab, 20, 2);
        let e3 = make_entry(&slab, 30, 3);

        unsafe {
            ws.push_entry(e1);
            ws.push_entry(e2);
            ws.push_entry(e3);
        }

        // Remove middle
        unsafe { ws.remove_entry(e2) };
        // Head should still be e1, e1.next = e3
        assert_eq!(ws.entry_head(), e1);
        unsafe {
            assert_eq!(entry_ref(e1).next(), e3);
            assert_eq!(entry_ref(e3).prev(), e1);
        }

        // Remove head
        unsafe { ws.remove_entry(e1) };
        assert_eq!(ws.entry_head(), e3);

        // Remove last
        unsafe { ws.remove_entry(e3) };
        assert!(ws.is_empty());

        unsafe {
            slab.free_ptr(e1);
            slab.free_ptr(e2);
            slab.free_ptr(e3);
        }
    }

    #[test]
    fn level_slot_index_computation() {
        // Level 0: shift=0, mask=63
        let lvl = Level::<u64>::new(64, 0, 3);
        assert_eq!(lvl.slot_index(0), 0);
        assert_eq!(lvl.slot_index(1), 1);
        assert_eq!(lvl.slot_index(63), 63);
        assert_eq!(lvl.slot_index(64), 0); // wraps
        assert_eq!(lvl.range(), 64);

        // Level 1: shift=3, mask=63
        let lvl1 = Level::<u64>::new(64, 1, 3);
        assert_eq!(lvl1.slot_index(0), 0);
        assert_eq!(lvl1.slot_index(8), 1); // 8 >> 3 = 1
        assert_eq!(lvl1.slot_index(64), 8); // 64 >> 3 = 8
        assert_eq!(lvl1.range(), 512);

        // Level 2: shift=6, mask=63
        let lvl2 = Level::<u64>::new(64, 2, 3);
        assert_eq!(lvl2.slot_index(64), 1); // 64 >> 6 = 1
        assert_eq!(lvl2.slot_index(512), 8); // 512 >> 6 = 8
        assert_eq!(lvl2.range(), 4096);
    }

    #[test]
    fn level_active_slot_list() {
        let lvl = Level::<u64>::new(64, 0, 3);

        assert!(lvl.active_head().is_null());

        let s0 = lvl.slot_ptr(0);
        let s5 = lvl.slot_ptr(5);

        // Link two slots
        unsafe {
            lvl.link_active(s0);
            lvl.link_active(s5);
        }

        // s5 is now the head (prepend)
        assert_eq!(lvl.active_head(), s5);
        unsafe {
            assert_eq!((*s5).active_next.get(), s0);
            assert!((*s0).active_next.get().is_null());
        }

        // Unlink s5 (head)
        unsafe { lvl.unlink_active(s5) };
        assert_eq!(lvl.active_head(), s0);

        // Unlink s0 (last)
        unsafe { lvl.unlink_active(s0) };
        assert!(lvl.active_head().is_null());
    }
}
