//! nexus-async-net — async adapters for nexus-net.
//!
//! Thin tokio wrappers over nexus-net's synchronous protocol primitives.
//! Same zero-copy parsing, same performance — just `.await` on I/O.
//!
//! # Modules
//!
//! - [`ws`] — Async WebSocket (wraps FrameReader/FrameWriter)
//! - [`rest`] — Async HTTP REST client (wraps RequestWriter/ResponseReader)

pub mod maybe_tls;
pub mod rest;
pub mod ws;

// Re-export nexus-net types for convenience
pub use nexus_net;
