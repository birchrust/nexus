//! Buffer primitives for network protocol parsing and framing.
//!
//! - [`ReadBuf`] — flat byte slab for inbound protocol parsing
//! - [`WriteBuf`] — flat byte slab for outbound protocol frames (sk_buff headroom model)

mod read_buf;
mod write_buf;

pub use read_buf::ReadBuf;
pub use write_buf::WriteBuf;
