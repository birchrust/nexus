# Assembly Codegen Audit

nexus-rt makes a bold claim: **zero-cost dispatch**. Monomorphized
type-level composition should let LLVM see through every abstraction and
produce codegen equivalent to hand-written inline code.

We verified this claim with 243 audit functions spanning every combinator,
adapter, and dispatch path in the framework.

## Methodology

Every audit function is marked `#[inline(never)]` to create a clean symbol
boundary. This prevents LLVM from folding the function into its caller,
giving us an honest view of what the framework produces in isolation.

The functions live in `nexus-rt/src/codegen_audit/` behind the
`codegen-audit` feature flag. They are not part of the public API — they
exist solely for codegen verification.

To inspect any function:

```bash
cargo asm --lib -p nexus-rt --features codegen-audit "pipe_linear_3"
```

Each function constructs a pipeline, DAG, or handler from scratch and runs
it once. The `#[inline(never)]` boundary means `cargo asm` shows exactly
what LLVM produces for the full lifecycle: build-time resolution + dispatch.

### What we're measuring

- **Inlining depth**: Does LLVM inline through the entire chain, or bail out
  partway?
- **Constant folding**: Does LLVM recognize algebraic identities across step
  boundaries (e.g., `add_one -> double -> add_three` = `2x + 5`)?
- **Branch elimination**: Are conditional combinators (guard, filter, route)
  compiled to branchless `cmov` instructions?
- **Dead code elimination**: Are side-effect-free combinators (tap, tee)
  removed entirely?
- **Copy elimination**: Are intermediate values kept in registers, or spilled
  to stack?
- **Adapter overhead**: Do wrapper types (ByRef, Cloned, Owned) add any
  instructions?

### Test categories

| # | Category | Functions | File |
|---|----------|-----------|------|
| 1-12 | Pipeline combinators | 105 | `pipeline.rs` |
| 13-20 | DAG chains, fork/merge | 65 | `dag.rs` |
| 21-22 | Batch pipeline and DAG | 34 | `batch.rs` |
| 23-25 | Adapters, handlers, templates | 24 | `adapters.rs` |
| 25+ | Stress / pathological | 15 | `stress.rs` |

Helper types and step functions shared across all files live in `helpers.rs`.

## Results

### Pipeline (categories 1-12)

| Category | What we tested | Grade | Key finding |
|----------|---------------|-------|-------------|
| Linear chains | 3/5/10 step pipelines | **A** | 10 steps compile to 8 ALU instructions. Full constant folding across step boundaries |
| Guard / filter | Predicate gating, early termination | **A** | Branchless `cmp + setae`. Guard failure skips entire dead chain via single `cmov` |
| Option chains | map / filter / unwrap_or / on_none / ok_or | **A** | Option discriminant tracked in registers. `unwrap_or` is branchless `cmov` |
| Result chains | map / map_err / catch / or_else / unwrap_or | **A** | Same register-based discriminant tracking. Error path never materializes `Result` on stack |
| Type transitions | u64 -> f64 -> u32 -> u8 -> u64 | **A** | 4 domain transitions evaporated completely when downstream doesn't observe intermediate types |
| Route / switch | 2-way route, 3-way switch, nested | **A** | Branchless `cmov` for route. Switch is 4 instructions |
| Splat | Tuple destructuring (2/3 elements) | **A** | Zero-cost register manipulation. Tuple never materialized on stack |
| Bool chains | and / or / xor combinators | **A** | Constant-folded to identity. Symbol aliasing (functions share same address) |
| Cloned | bare / Option / Result / .cloned() | **A** | Copy types: zero-cost. Non-copy: single `clone` call, no framework overhead |
| Tap / tee / dedup | Side-effect combinators | **A** | Tap: dead-code eliminated to 1 instruction. Dedup: `cmp + cmov` |
| Dispatch | fan_out, broadcast, mid-chain | **B+** | Handler body inlined. Build-time resolution not inlined (expected, cold-path) |
| World access | Res / ResMut / Local / changed_after | **A** | Direct pointer deref — 1 load per resource. Change detection: `cmp` + branchless `shl` |

Pure computation combinators achieve **A** — LLVM treats them as if they
don't exist. World-accessing combinators achieve **A** — ResourceId is a
direct pointer, so the cost is a single deref per resource, not the
framework.

### DAG (categories 13-20)

| Category | What we tested | Grade | Key finding |
|----------|---------------|-------|-------------|
| Linear DAG | 2/3/4 step chains | **A** | Identical quality to pipeline. Reference-passing is zero-cost |
| Fork / merge | 2-way, 3-way, nested, asymmetric | **A** | Fork/merge eliminated at the type level. `2x + 3x` strength-reduced to `lea [rbx + 4*rbx]` |
| Filter / guard | Guard in chain, filter in arm | **A** | Same branchless `cmov` pattern as pipeline |
| Option / Result | DAG-specific option/result chains | **A** | Discriminants tracked in registers across fork boundaries |
| Route in DAG | resolve_arm + switch pattern | **A** | Multi-way dispatch via `cmp + cmovae` chains |
| Splat in DAG | Merge-step destructuring | **A** | Reuses existing MergeStepCall infrastructure with zero additional cost |
| Dispatch in DAG | fan_out at DAG leaf | **B+** | Same pattern as pipeline dispatch |
| Cloned in DAG | Reference -> value adaptation | **A** | `cloned()` on reference output types is zero-cost for Copy types |

Nested fork/merge (2 levels deep) produces identical instruction count to a
flat chain. LLVM doesn't just inline the fork — it algebraically combines
the arm computations:

```asm
; double -> fork(add_one, triple) -> merge_add  =>  8x + 1
; outer merge with add_seven                    =>  9x + 8
lea r8, [rbx + 8*rbx]    ; 9 * input
add rdi, 17               ; all constants folded
```

### Batch (categories 21-22)

| Category | What we tested | Grade | Key finding |
|----------|---------------|-------|-------------|
| Batch pipeline | Linear, guard, switch, dispatch, empty, single-item | **A-** | Inner loops: tight, indexed, 2x unrolled. Per-item cost identical to single-item |
| Batch DAG | Fork/merge, nested, option/result, heavy arms | **A-** | Fork/merge in batch loop: same `5x` strength reduction as single-item |

Minor deduction for Vec allocation (`__rust_alloc`), which is inherent to
the batch API. Inner loop bodies are optimal. Empty batch compiles to 6
instructions.

### Adapters and handlers (categories 23-25)

| Category | What we tested | Grade | Key finding |
|----------|---------------|-------|-------------|
| Adapters | ByRef, Cloned, Owned, Adapt, nested | **A** | ByRef: completely eliminated (6 instructions total). Cloned(ByRef(inner)): zero-cost round-trip |
| Combinators | fan_out (2/4/8), broadcast (2/3/4) | **A** | fan_out: fully inlined and dead-code eliminated when no side effects |
| Handlers / templates | IntoHandler (0-3 params), virtual, catch_unwind, HandlerTemplate, CallbackTemplate | **A-** | `Box<dyn Handler>` devirtualized when concrete type locally known. Template: 1 indirect call per stamp |

`Box<dyn Handler<u64>>` with a locally-known concrete type produces **no
vtable dispatch**. LLVM sees through the boxing and inlines the handler
body directly. The only overhead is alloc + dealloc for the box itself.

### Stress tests

| Test | Steps | Instructions | Grade | Key finding |
|------|-------|-------------|-------|-------------|
| 30-step pipeline | 30 | 21 | **A+** | Pure register computation. No stack frame. No calls. Algebraic simplification across all 30 steps |
| 50-step pipeline | 50 | 33 | **A+** | No inlining threshold hit. Linear scaling: +12 instructions for +20 steps |
| Large type (4096 B) | 3 | 17 | **B+** | No memcpy between steps. LLVM writes directly into caller's sret buffer |
| Kitchen sink (16 combinators) | 16 | 66 | **A-** | Every combinator inlined. Instruction count reflects algorithmic complexity, not overhead |

The 50-step result is particularly important: it confirms LLVM's inlining
budget holds for deep monomorphized chains with no signs of collapse.

## Why it works

The architectural decision enabling this codegen is **type-level chain
composition** using **named chain node types**. Each pipeline or DAG builder
produces a unique monomorphized type encoding the entire chain — the same
pattern as iterator adapters (`Map<Filter<Iter, P>, F>`):

```text
Pipeline<ThenNode<ThenNode<IdentityNode, Step<add_one>>, Step<double>>>
```

LLVM sees the entire computation as a single function body. There are no
trait objects, no indirect calls, no function pointers in the dispatch
chain. Every node's `ChainCall::call()` is inlined at the same optimization
level as if you'd written:

```rust
let x = add_one(input);
let x = double(x);
let x = add_three(x);
consume(world, x);
```

This is exactly what LLVM produces — and then it goes further, folding
adjacent operations algebraically.

## Where it's not perfect

**Build-time initialization** (`IntoHandler::into_handler`,
`IntoStep::into_step`) is not inlined. These resolve ResourceIds via
HashMap lookup at construction time. This is cold-path by design — it
happens once at setup, not per-event.

**Template dispatch** uses indirect `call` through a function pointer. The
template pattern erases the function type to enable N-stamp amortization.
The fn pointer is stable (same target for all stamps), so branch prediction
hits after the first call.

**High-arity handlers** (5+ resource params) pay for each resource fetch
at dispatch time. Each `Res<T>` / `ResMut<T>` costs one pointer deref
(the `ResourceId` IS the pointer). For 8 params, that's 8 independent
loads — pipelineable by out-of-order execution. Zero framework overhead;
the loads are the inherent cost of indirected access.

None of these are framework overhead — they're the actual cost of the
operations being performed.
