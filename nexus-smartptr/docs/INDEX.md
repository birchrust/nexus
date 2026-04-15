# nexus-smartptr documentation

Inline smart pointers for `?Sized` types. `Flat<T, B>` stores trait objects
in an inline buffer; `Flex<T, B>` does the same with heap fallback. Avoids
mandatory boxing for type-erased storage.

## Contents

- [overview.md](overview.md) — the problem and the `dyn Trait` without
  `Box` pattern
- [flat.md](flat.md) — `Flat<T, B>`, fixed-capacity inline storage
- [flex.md](flex.md) — `Flex<T, B>`, inline with heap fallback
- [caveats.md](caveats.md) — the fat-pointer layout assumption, `build.rs`
  validation, and what can go wrong
- [patterns.md](patterns.md) — cookbook: handler collections, no-alloc
  trait object storage

## Related crates

- [`nexus-rt`](../../nexus-rt) — consumes `Flat`/`Flex` for its handler
  storage (behind the `smartptr` feature)
- `smallbox` — similar design; `nexus-smartptr` takes the "capacity is
  the type" approach instead of "capacity is a const generic number"
