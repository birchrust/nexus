//! Category-based imports for nexus-stats types.
//!
//! All types remain available at the crate root. These modules provide
//! optional namespaced imports for projects that prefer categorical organization.
//!
//! ```rust
//! // Both work:
//! use nexus_stats::LinearRegressionF64;
//! use nexus_stats::categories::regression::LinearRegressionF64 as _;
//! ```

pub mod bayesian;
pub mod change;
pub mod estimation;
pub mod filters;
pub mod hypothesis;
pub mod info;
pub mod optimization;
pub mod regression;
pub mod signal;
pub mod smoothing;
