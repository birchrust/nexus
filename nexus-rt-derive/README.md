# nexus-rt-derive

Proc-macro crate for [nexus-rt](https://crates.io/crates/nexus-rt).

**Do not depend on this crate directly.** Use `nexus-rt` instead — the
derives are re-exported as `nexus_rt::{Resource, Param, Deref, DerefMut}`.

## Provides

- `#[derive(Resource)]` — marker trait for World-storable types
- `#[derive(Param)]` — derive `SystemParam` for custom handler parameters
- `#[derive(Deref)]` / `#[derive(DerefMut)]` — newtype delegation

See the [nexus-rt documentation](https://docs.rs/nexus-rt) for usage.
