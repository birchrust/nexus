//! nexus-async-net — async adapters for nexus-net.
//!
//! Thin async wrappers over nexus-net's synchronous protocol primitives.
//! Same zero-copy parsing, same performance — just `.await` on I/O.
//!
//! # Runtime Features
//!
//! Exactly one async runtime must be enabled (mutually exclusive):
//!
//! - **`tokio-rt`** (default) — tokio-based adapters for WebSocket and REST.
//! - **`nexus`** — nexus-async-rt-based adapters (single-threaded, pre-allocated).
//!   *(Renamed from `nexus-rt` in v0.4.2.)*
//!
//! # Modules
//!
//! - [`ws`] — Async WebSocket (wraps FrameReader/FrameWriter) — `tokio-rt` only
//! - [`rest`] — Async HTTP REST client (wraps RequestWriter/ResponseReader)

#[cfg(all(feature = "tokio-rt", feature = "nexus"))]
compile_error!("features `tokio-rt` and `nexus` are mutually exclusive — pick one async runtime");

pub(crate) mod maybe_tls;
pub mod rest;
pub mod ws;

// Re-export nexus-net types for convenience
pub use nexus_net;
