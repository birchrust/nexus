//! Timer wheel entry — the node stored in each slab slot.
//!
//! Each entry carries DLL links (for the per-slot entry list), a deadline,
//! a lightweight refcount, and the user's value wrapped in `UnsafeCell<Option<T>>`.

use std::cell::{Cell, UnsafeCell};
use std::ptr;

use nexus_slab::SlotCell;

/// Raw pointer to a `SlotCell<WheelEntry<T>>`.
///
/// This is the currency of the intrusive DLL — entries link to each other
/// through their slab slot pointers.
pub(crate) type EntryPtr<T> = *mut SlotCell<WheelEntry<T>>;

/// Sentinel for null entry pointers.
/// Returns a null `EntryPtr<T>`.
#[inline]
pub(crate) fn null_entry<T>() -> EntryPtr<T> {
    ptr::null_mut()
}

/// Timer wheel entry stored inside a slab slot.
///
/// This type appears in the public type signature of [`TimerWheel`](crate::TimerWheel)
/// as the slab's item type, but is opaque — users never construct or access it directly.
///
/// # Layout
///
/// DLL links and deadline are hot (touched every list walk / poll check).
/// The refcount is accessed on schedule/cancel/fire. Level and slot index
/// are set at insertion and read on cancel — they replace the recomputation
/// that was unsound after time advances. The value is cold relative to the
/// links — only read on fire or cancel.
///
/// `UnsafeCell<Option<T>>` provides interior mutability for `.take()` through
/// shared references (slab gives `&self` access). `Option<T>` makes Drop safe:
/// `drop_in_place` on `None` is a no-op. Single-threaded (`!Send`), so
/// `UnsafeCell` is sound.
#[repr(C)]
pub struct WheelEntry<T> {
    // DLL links — touched every list walk (hot)
    prev: Cell<EntryPtr<T>>,
    next: Cell<EntryPtr<T>>,
    // Deadline — compared during poll (hot)
    deadline_ticks: u64,
    // Refcount — 1=wheel only (fire-and-forget), 2=wheel+handle
    refs: Cell<u8>,
    // Location — set at insertion, read at cancel. Avoids recomputing level
    // from delta (which is unsound after current_ticks advances).
    level: Cell<u8>,
    slot_idx: Cell<u16>,
    // Value — read only on fire/cancel (cold relative to links)
    value: UnsafeCell<Option<T>>,
}

impl<T> WheelEntry<T> {
    /// Creates a new entry with the given deadline and value.
    ///
    /// `refs` starts at the given count (1 for fire-and-forget, 2 for handle-bearing).
    #[inline]
    pub(crate) fn new(deadline_ticks: u64, value: T, refs: u8) -> Self {
        WheelEntry {
            prev: Cell::new(null_entry()),
            next: Cell::new(null_entry()),
            deadline_ticks,
            refs: Cell::new(refs),
            level: Cell::new(0),
            slot_idx: Cell::new(0),
            value: UnsafeCell::new(Some(value)),
        }
    }

    // =========================================================================
    // DLL link accessors
    // =========================================================================

    #[inline]
    pub(crate) fn prev(&self) -> EntryPtr<T> {
        self.prev.get()
    }

    #[inline]
    pub(crate) fn next(&self) -> EntryPtr<T> {
        self.next.get()
    }

    #[inline]
    pub(crate) fn set_prev(&self, ptr: EntryPtr<T>) {
        self.prev.set(ptr);
    }

    #[inline]
    pub(crate) fn set_next(&self, ptr: EntryPtr<T>) {
        self.next.set(ptr);
    }

    // =========================================================================
    // Deadline
    // =========================================================================

    #[inline]
    pub(crate) fn deadline_ticks(&self) -> u64 {
        self.deadline_ticks
    }

    // =========================================================================
    // Refcount
    // =========================================================================

    #[inline]
    pub(crate) fn refs(&self) -> u8 {
        self.refs.get()
    }

    /// Decrements the refcount by 1 and returns the new value.
    #[inline]
    pub(crate) fn dec_refs(&self) -> u8 {
        let r = self.refs.get() - 1;
        self.refs.set(r);
        r
    }

    // =========================================================================
    // Location (level + slot index) — set at insertion, read at cancel
    // =========================================================================

    /// Returns the level index this entry was placed in.
    #[inline]
    pub(crate) fn level(&self) -> u8 {
        self.level.get()
    }

    /// Returns the slot index within the level this entry was placed in.
    #[inline]
    pub(crate) fn slot_idx(&self) -> u16 {
        self.slot_idx.get()
    }

    /// Records the level and slot index this entry was placed in.
    #[inline]
    pub(crate) fn set_location(&self, level: u8, slot_idx: u16) {
        self.level.set(level);
        self.slot_idx.set(slot_idx);
    }

    // =========================================================================
    // Value access
    // =========================================================================

    /// Takes the value out, leaving `None` in its place.
    ///
    /// # Safety
    ///
    /// Must be called from a single thread (enforced by `!Send` on the wheel).
    #[inline]
    pub(crate) unsafe fn take_value(&self) -> Option<T> {
        // SAFETY: single-threaded access guaranteed by !Send on TimerWheel
        unsafe { (*self.value.get()).take() }
    }
}

/// Dereferences an `EntryPtr<T>` to a `&WheelEntry<T>`.
///
/// # Safety
///
/// - `ptr` must be non-null.
/// - `ptr` must point to an occupied `SlotCell<WheelEntry<T>>` within a live slab.
/// - The returned reference must not outlive the slab allocation (i.e. must not
///   be held across a `free_ptr` call on the same slot).
#[inline]
pub(crate) unsafe fn entry_ref<'a, T>(ptr: EntryPtr<T>) -> &'a WheelEntry<T> {
    // SAFETY: SlotCell is a union; when occupied, the `value` field is active.
    // The value is `ManuallyDrop<MaybeUninit<WheelEntry<T>>>`, and we know it's
    // initialized because the slot was allocated via slab.alloc/try_alloc.
    unsafe { (*ptr).value.assume_init_ref() }
}
