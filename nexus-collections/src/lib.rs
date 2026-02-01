//! High-performance collections with claim-based ownership.
//!
//! Collections own data, users hold claims (non-Copy handles) for O(1) access
//! and removal.
//!
//! # Design Philosophy: The Bank Account Analogy
//!
//! - **Collection** = Bank (holds the money/data)
//! - **Claim** = Account (your right to access it)
//! - **Data** = Money (what you actually care about)
//!
//! You own the account, not the money. The bank holds the money on your behalf.
//! When you close the account (remove), you get your money back. Claims are not
//! Copy because two people shouldn't share one bank account.

#![warn(missing_docs)]

mod internal;

pub mod list;

// TODO: Phase 3-5 - Implement heap, skip modules and remove old storage
// pub mod heap;
// pub mod skiplist;
// pub mod storage;

pub use list::{
    BoundedListSlab, DetachedListNode, Id as ListId, List, ListSlabOps, ListSlot,
    Node as ListNode, UnboundedListSlab,
};

// Re-export sealed traits for generic bounds
pub use internal::SlotOps;
