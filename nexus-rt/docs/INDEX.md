# nexus-rt Documentation

Event-driven runtime with zero-cost dispatch.

## User Guide

- [Getting Started](getting-started.md) — Build your first nexus-rt application
- [World & Resources](world.md) — The resource model: registration, access, lifetimes
- [Handlers](handlers.md) — Writing handlers, parameter resolution, named functions
- [Pipelines & DAGs](pipelines.md) — Composing processing chains
- [Drivers](drivers.md) — The installer/poller pattern for IO and timers
- [Clock](clock.md) — Time sources: realtime, test, historical
- [Templates](templates.md) — Advanced: blueprints for handler factories

## Internal Reference

These documents explain internal design decisions for contributors:

- [Annotation Traits](annotation-traits.md) — Compile-time config via marker traits
- [Chain Types](chain-types.md) — Why pipelines use named types, not closures
- [Codegen Audit](codegen-audit.md) — Assembly verification of zero-cost dispatch
