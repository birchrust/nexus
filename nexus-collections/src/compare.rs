//! Comparison strategy for tree key ordering.
//!
//! Decouples sort order from the key type, allowing the same key type
//! (e.g., `Price`) to be sorted ascending in one tree and descending
//! in another — sharing the same node type and allocator.

use std::cmp::Ordering;

/// Comparison strategy for tree key ordering.
///
/// # Examples
///
/// ```ignore
/// use nexus_collections::compare::{Compare, Reverse};
///
/// // Bid side: highest price first
/// let mut bids = levels::RbTree::<Reverse>::new(levels::Allocator);
///
/// // Ask side: lowest price first (default Natural ordering)
/// let mut asks = levels::RbTree::new(levels::Allocator);
/// ```
pub trait Compare<K> {
    /// Compares two keys.
    fn cmp(a: &K, b: &K) -> Ordering;
}

/// Natural ordering — delegates to `K: Ord`.
pub struct Natural;

impl<K: Ord> Compare<K> for Natural {
    #[inline(always)]
    fn cmp(a: &K, b: &K) -> Ordering {
        a.cmp(b)
    }
}

/// Reverse ordering — `b.cmp(a)`.
pub struct Reverse;

impl<K: Ord> Compare<K> for Reverse {
    #[inline(always)]
    fn cmp(a: &K, b: &K) -> Ordering {
        b.cmp(a)
    }
}
