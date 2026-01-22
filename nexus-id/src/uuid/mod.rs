//! UUID generators optimized for high-performance systems.
//!
//! This module provides UUID v4 and v7 generators that avoid syscalls on the
//! hot path by seeding a fast PRNG once at construction.
//!
//! # UUID Versions
//!
//! | Version | Structure | Use Case |
//! |---------|-----------|----------|
//! | [`UuidV4`] | 122 random bits | "Just give me a unique ID" |
//! | [`UuidV7`] | Timestamp + random | Time-ordered, better for DBs/logs |
//!
//! # Performance
//!
//! Both generators achieve ~40-50 cycles per UUID including string formatting:
//! - One `getrandom` syscall at construction (or explicit seed)
//! - Zero syscalls on the hot path
//! - Zero allocation (outputs to stack-allocated `AsciiString`)
//!
//! # Example
//!
//! ```rust
//! use std::time::{Instant, SystemTime, UNIX_EPOCH};
//! use nexus_id::uuid::{UuidV4, UuidV7};
//!
//! // V4: Just needs a seed
//! let mut v4 = UuidV4::from_entropy();
//! let id = v4.next();  // "a1b2c3d4-e5f6-4789-abcd-ef1234567890"
//!
//! // V7: Needs epoch for timestamp tracking
//! let epoch = Instant::now();
//! let unix_base = SystemTime::now()
//!     .duration_since(UNIX_EPOCH)
//!     .unwrap()
//!     .as_millis() as u64;
//! let mut v7 = UuidV7::from_entropy(epoch, unix_base);
//! let id = v7.next(Instant::now());  // "0190a5e4-b876-7abc-8def-1234567890ab"
//! ```

mod v4;
mod v7;

pub use v4::UuidV4;
pub use v7::UuidV7;
