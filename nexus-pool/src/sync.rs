//! Thread-safe object pools.
//!
//! Currently provides:
//! - [`BoundedSingleAcquirerPool`]: One thread acquires, any thread returns
//!
//! # Example
//!
//! ```
//! use nexus_pool::sync::BoundedSingleAcquirerPool;
//!
//! let pool = BoundedSingleAcquirerPool::new(
//!     100,
//!     || Vec::<u8>::with_capacity(1024),
//!     |v| v.clear(),
//! );
//!
//! let acquirer = pool.into_acquirer();
//!
//! // Acquire on this thread
//! let mut buf = acquirer.try_acquire().unwrap();
//! buf.extend_from_slice(b"hello");
//!
//! // Send to another thread - returns to pool when dropped
//! std::thread::spawn(move || {
//!     println!("{:?}", &*buf);
//! }).join().unwrap();
//! ```

mod single_acquire;

pub use single_acquire::{SingleAcquirer, BoundedSingleAcquirerPool, SingleAcquirerPooled};
