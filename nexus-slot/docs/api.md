# API reference

## SPSC

```rust
use nexus_slot::spsc;

#[derive(Copy, Clone, Default)]
struct Quote { bid: f64, ask: f64, seq: u64 }

let (mut writer, mut reader) = spsc::slot::<Quote>();
```

`slot::<T>()` returns `(Writer<T>, Reader<T>)`. Both are `Send` but not
`Sync` — each belongs to exactly one thread.

### `Writer::write`

```rust
# use nexus_slot::spsc;
# #[derive(Copy, Clone, Default)] struct Quote { bid: f64, ask: f64, seq: u64 }
# let (mut writer, _reader) = spsc::slot::<Quote>();
writer.write(Quote { bid: 100.0, ask: 100.05, seq: 1 });
```

Always succeeds. Overwrites whatever was in the slot. Internally: bump
sequence, atomic store of bytes, bump sequence.

### `Reader::read` / `read_versioned`

```rust
# use nexus_slot::spsc;
# #[derive(Copy, Clone, Default)] struct Quote { bid: f64, ask: f64, seq: u64 }
# let (mut writer, mut reader) = spsc::slot::<Quote>();
# writer.write(Quote { bid: 100.0, ask: 100.05, seq: 1 });
// Returns None if no new value has been written since the last read.
if let Some(q) = reader.read() {
    // First read after a write — consume it.
}

// Same but also returns the sequence number.
if let Some((q, version)) = reader.read_versioned() {
    // version monotonically increases per write.
}
```

`read()` only returns `Some` when the slot contains a value the reader
hasn't seen yet. Call it in a loop until it returns `None` to drain all
pending updates (there will be at most one — the last write wins).

### `Reader::has_update`

```rust
# use nexus_slot::spsc;
# #[derive(Copy, Clone, Default)] struct Quote { bid: f64, ask: f64, seq: u64 }
# let (mut _writer, reader) = spsc::slot::<Quote>();
if reader.has_update() {
    // There is a new value to read.
}
```

Lock-free check. Use it to gate a `read()` call when you have other work
to do.

### Disconnection

```rust
# use nexus_slot::spsc;
# #[derive(Copy, Clone, Default)] struct Quote { bid: f64, ask: f64, seq: u64 }
# let (writer, reader) = spsc::slot::<Quote>();
assert!(!writer.is_disconnected());
drop(reader);
assert!(writer.is_disconnected());
```

Dropping one side signals the other. The remaining side can observe
disconnection and shut down gracefully. Unlike a channel, disconnection
doesn't invalidate pending data — the writer can still `write()` after
the reader drops; it's just pointless.

## SPMC

```rust
use nexus_slot::spmc;

#[derive(Copy, Clone, Default)]
struct Quote { bid: f64, ask: f64, seq: u64 }

let (mut writer, mut reader1) = spmc::shared_slot::<Quote>();
let mut reader2 = reader1.clone();
let mut reader3 = reader1.clone();
```

`shared_slot::<T>()` returns `(Writer<T>, SharedReader<T>)`.
`SharedReader: Clone` — each clone is an independent consumer.

```rust
# use nexus_slot::spmc;
# #[derive(Copy, Clone, Default)] struct Quote { bid: f64, ask: f64, seq: u64 }
# let (mut writer, mut reader1) = spmc::shared_slot::<Quote>();
# let mut reader2 = reader1.clone();
writer.write(Quote { bid: 100.0, ask: 100.05, seq: 1 });

// Both readers see the same value exactly once.
assert_eq!(reader1.read().unwrap().seq, 1);
assert_eq!(reader2.read().unwrap().seq, 1);

// Neither sees it again until the writer writes something new.
assert!(reader1.read().is_none());
```

Each `SharedReader` tracks its own last-seen version, so fan-out is
independent — a slow reader doesn't hold up fast readers.

### Number of readers

Readers can be created (by cloning) and dropped at runtime. The writer
doesn't know or care how many readers exist. Drop the last reader and the
writer sees `is_disconnected()`.

## The `Pod` trait

```rust
pub unsafe trait Pod: Sized { /* ... */ }

unsafe impl<T: Copy> Pod for T {}  // blanket impl for Copy types
```

To implement for a non-`Copy` type:

```rust
use nexus_slot::Pod;

#[repr(C)]
pub struct Frame {
    pub data: [u8; 64],
    pub len: u16,
    pub flags: u16,
}

// SAFETY: Frame contains only POD fields, no drop glue, no heap pointers.
unsafe impl Pod for Frame {}
```

The trait has a compile-time assertion that
`std::mem::needs_drop::<Self>() == false`. If you accidentally implement
`Pod` for a type containing `String` or `Vec`, you get a compile error.

## Memory ordering

The seqlock uses Release on writer side (after the byte copy), Acquire on
reader side (before and after the byte copy). This means a successful
`read()` establishes a happens-before relationship with the `write()`
that produced the value — ordinary data inside `T` is safely visible.
