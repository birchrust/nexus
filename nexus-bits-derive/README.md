# nexus-bits-derive

Proc-macro crate for [nexus-bits](https://crates.io/crates/nexus-bits).

**Do not depend on this crate directly.** Use `nexus-bits` instead — the
derive macros are re-exported automatically.

## Provides

- `#[derive(BitStruct)]` — flat bit-packed structs
- `#[derive(BitEnum)]` — tagged unions with discriminant and per-variant fields
- `#[derive(IntEnum)]` — simple integer-backed enums

See the [nexus-bits documentation](https://docs.rs/nexus-bits) for usage.
