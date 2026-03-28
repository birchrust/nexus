//! nexus-net — low-latency network protocol primitives.
//!
//! Sans-IO protocol implementations that operate on byte slices.
//! No async runtime, no I/O layer — pure protocol state machines.
//!
//! # Modules
//!
//! - [`buf`] — Buffer primitives (ReadBuf, WriteBuf)
//! - [`ws`] — WebSocket framing (RFC 6455)

pub mod buf;
pub mod http;
pub mod ws;
