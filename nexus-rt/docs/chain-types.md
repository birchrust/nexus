# Named Chain Types

Pipeline and DAG chains are composed using named struct types following the
iterator adapter pattern (`Map<Filter<Iter, P>, F>`). Each combinator wraps
the previous chain in a new node struct, producing a fully monomorphized
type the compiler can see through completely.

## Why named types (not closures)

The original implementation composed chains using closures — each `.then()`
created a closure capturing the previous chain and the new step:

```text
// Old: unnameable closure types
impl FnMut(&mut World, In) -> Out + use<In, Out, ...>
```

This hit a wall with **HRTB (Higher-Ranked Trait Bounds)**. Rust 2024's
precise capturing (`use<>`) bakes `In`'s lifetime into the closure type.
When `In = &'a T`, the closure type is tied to `'a` — making
`Box<dyn for<'a> Handler<&'a T>>` impossible.

Named types solve this because `In` appears only on the trait impl, not on
the struct definition:

```rust
pub struct ThenNode<Prev, S> { prev: Prev, step: S }

impl<In, Prev: ChainCall<In>, S: StepCall<Prev::Out>> ChainCall<In>
    for ThenNode<Prev, S>
{
    type Out = S::Out;
    fn call(&mut self, world: &mut World, input: In) -> Self::Out {
        let mid = self.prev.call(world, input);
        self.step.call(world, mid)
    }
}
```

Since `ThenNode<Prev, S>` doesn't mention `In`, a `Pipeline<ThenNode<...>>`
can implement `for<'a> Handler<&'a T>` — enabling zero-copy event dispatch
with borrowed data in boxed handlers.

## Core traits

| Trait | Role | Defined in |
|-------|------|-----------|
| `ChainCall<In>` | Main chain composition (pipeline + DAG main chain) | `pipeline.rs` |
| `ArmChainCall<In>` | DAG arm composition (input by `&In`) | `dag.rs` |
| `StepCall<In>` | Pre-resolved step (associated `Out`) | `pipeline.rs` |
| `RefStepCall<In>` | Pre-resolved step taking `&In` | `pipeline.rs` |
| `ProducerCall` | Pre-resolved no-input producer | `pipeline.rs` |
| `ScanStepCall<Acc, In>` | Pre-resolved scan step | `pipeline.rs` |
| `RefScanStepCall<Acc, In>` | Pre-resolved scan step taking `&In` | `pipeline.rs` |
| `MergeStepCall<In, Out>` | DAG merge step (tuple of arm refs -> Out) | `dag.rs` |

`StepCall` and friends have associated `Out` types — critical for HRTB since
it avoids baking intermediate output types (which may contain lifetimes from
`In`) into the struct definition.

## Node type catalogue

All node types are `#[doc(hidden)]` with `pub(crate)` fields. Users never
construct or name these — they appear only in inferred return types.

### Pipeline chain nodes (ChainCall<In>)

#### Core (Out = T)

| Node | Combinator | Transform |
|------|-----------|-----------|
| `IdentityNode` | (start) | T -> T passthrough |
| `ThenNode<Prev, S>` | `.then()` | T -> S::Out (by value) |
| `TapNode<Prev, S>` | `.tap()` | T -> T (side effect via &T) |
| `GuardNode<Prev, S>` | `.guard()` | T -> Option\<T\> |
| `DedupNode<Prev>` | `.dedup()` | T -> Option\<T\> |
| `ScanNode<Prev, S, Acc>` | `.scan()` | T -> S::Out (with &mut Acc) |
| `RefScanNode<Prev, S, Acc>` | `.scan()` (ref variant) | &T -> S::Out (with &mut Acc) |
| `DispatchNode<Prev, H>` | `.dispatch()` | T -> () (feeds Handler) |
| `TeeNode<Prev, H>` | `.tee()` | T -> T (clone to Handler) |
| `RouteNode<Prev, P, A, B>` | `.route()` | T -> () (binary dispatch) |

#### Option\<T\>

| Node | Combinator | Transform |
|------|-----------|-----------|
| `MapOptionNode<Prev, S>` | `.map()` | Option\<T\> -> Option\<S::Out\> |
| `FilterNode<Prev, S>` | `.filter()` | Option\<T\> -> Option\<T\> |
| `InspectOptionNode<Prev, S>` | `.inspect()` | Option\<T\> -> Option\<T\> |
| `AndThenNode<Prev, S>` | `.and_then()` | Option\<T\> -> Option\<U\> |
| `OnNoneNode<Prev, P>` | `.on_none()` | Option\<()\> -> () |
| `OkOrNode<Prev, E>` | `.ok_or()` | Option\<T\> -> Result\<T,E\> |
| `OkOrElseNode<Prev, P>` | `.ok_or_else()` | Option\<T\> -> Result\<T,E\> |
| `UnwrapOrOptionNode<Prev>` | `.unwrap_or()` | Option\<T\> -> T |
| `UnwrapOrElseOptionNode<Prev, P>` | `.unwrap_or_else()` | Option\<T\> -> T |

#### Result\<T, E\>

| Node | Combinator | Transform |
|------|-----------|-----------|
| `MapResultNode<Prev, S>` | `.map()` | Result\<T,E\> -> Result\<S::Out,E\> |
| `AndThenResultNode<Prev, S>` | `.and_then()` | Result\<T,E\> -> Result\<U,E\> |
| `CatchNode<Prev, S>` | `.catch()` | Result\<T,E\> -> Option\<T\> |
| `MapErrNode<Prev, S>` | `.map_err()` | Result\<T,E\> -> Result\<T,E2\> |
| `OrElseNode<Prev, S>` | `.or_else()` | Result\<T,E\> -> Result\<T,E2\> |
| `InspectResultNode<Prev, S>` | `.inspect()` | Result\<T,E\> -> Result\<T,E\> |
| `InspectErrNode<Prev, S>` | `.inspect_err()` | Result\<T,E\> -> Result\<T,E\> |
| `OkResultNode<Prev>` | `.ok()` | Result\<T,E\> -> Option\<T\> |
| `UnwrapOrResultNode<Prev>` | `.unwrap_or()` | Result\<T,E\> -> T |
| `UnwrapOrElseResultNode<Prev, S>` | `.unwrap_or_else()` | Result\<T,E\> -> T |

#### Bool

| Node | Combinator | Transform |
|------|-----------|-----------|
| `NotNode<Prev>` | `.not()` | bool -> bool |
| `AndBoolNode<Prev, P>` | `.and()` | bool -> bool |
| `OrBoolNode<Prev, P>` | `.or()` | bool -> bool |
| `XorBoolNode<Prev, P>` | `.xor()` | bool -> bool |

#### Cloned

| Node | Combinator | Transform |
|------|-----------|-----------|
| `ClonedNode<Prev>` | `.cloned()` on &T | &T -> T |
| `ClonedOptionNode<Prev>` | `.cloned()` on Option<&T> | Option\<&T\> -> Option\<T\> |
| `ClonedResultNode<Prev>` | `.cloned()` on Result<&T,E> | Result\<&T,E\> -> Result\<T,E\> |

#### Terminal

| Node | Combinator | Transform |
|------|-----------|-----------|
| `DiscardOptionNode<Prev>` | `.build()` on Option\<()\> | Option\<()\> -> () |

### DAG-specific chain nodes

These nodes exist because DAG steps take input by reference (`&T`) rather
than by value. They implement both `ChainCall<In>` (for main chain) and
`ArmChainCall<In>` (for arms).

| Node | Combinator | Transform |
|------|-----------|-----------|
| `DagThenNode<Prev, S, NewOut>` | `.then()` in DAG | borrows &Prev::Out -> NewOut |
| `DagMapOptionNode<Prev, S, NewOut>` | `.map()` on Option in DAG | Option borrows &T -> Option\<NewOut\> |
| `DagMapResultNode<Prev, S, NewOut>` | `.map()` on Result in DAG | Result borrows &T -> Result\<NewOut,E\> |
| `DagAndThenOptionNode<Prev, S, NewOut>` | `.and_then()` on Option in DAG | Option borrows &T -> Option\<NewOut\> |
| `DagAndThenResultNode<Prev, S, NewOut>` | `.and_then()` on Result in DAG | Result borrows &T -> Result\<NewOut,E\> |
| `DagCatchNode<Prev, S>` | `.catch()` on Result in DAG | borrows &E -> Option\<T\> |
| `DagRouteNode<Prev, P, A, B>` | `.route()` in DAG | borrows &T for predicate + arms |

### DAG topology nodes

| Node | Use | Transform |
|------|-----|-----------|
| `MergeNode2<Chain, C0, C1, MS, ...>` | `.merge()` (2 arms) | chain -> fork -> 2 arms -> merge step |
| `MergeNode3<Chain, C0, C1, C2, MS, ...>` | `.merge()` (3 arms) | chain -> fork -> 3 arms -> merge step |
| `MergeNode4<Chain, C0, C1, C2, C3, MS, ...>` | `.merge()` (4 arms) | chain -> fork -> 4 arms -> merge step |
| `JoinNode2<Chain, C0, C1, ...>` | `.join()` (2 arms) | chain -> fork -> 2 sink arms -> () |
| `JoinNode3<Chain, C0, C1, C2, ...>` | `.join()` (3 arms) | chain -> fork -> 3 sink arms -> () |
| `JoinNode4<Chain, C0, C1, C2, C3, ...>` | `.join()` (4 arms) | chain -> fork -> 4 sink arms -> () |

### Splat nodes (tuple destructuring)

Defined per arity (2-5) inside the `define_dag_splat_builders!` macro.

| Node | Arity | Use |
|------|-------|-----|
| `SplatThenNode2<Chain, MS, T0, T1, NewOut>` | 2 | Pipeline/DAG splat .then() |
| `SplatThenNode3<...>` | 3 | Pipeline/DAG splat .then() |
| `SplatThenNode4<...>` | 4 | Pipeline/DAG splat .then() |
| `SplatThenNode5<...>` | 5 | Pipeline/DAG splat .then() |
| `SplatArmStartNode2<S, T0, T1, Out>` | 2 | DAG arm start after splat |
| `SplatArmStartNode3<...>` | 3 | DAG arm start after splat |
| `SplatArmStartNode4<...>` | 4 | DAG arm start after splat |
| `SplatArmStartNode5<...>` | 5 | DAG arm start after splat |

### Shared nodes (dual impls)

Most pipeline nodes also implement `ArmChainCall<In>` for use in DAG arms.
The following nodes have both `ChainCall<In>` and `ArmChainCall<In>` impls:

`TapNode`, `GuardNode`, `DedupNode`, `FilterNode`, `InspectOptionNode`,
`InspectResultNode`, `InspectErrNode`, `OnNoneNode`, `OkOrNode`,
`OkOrElseNode`, `UnwrapOrOptionNode`, `UnwrapOrElseOptionNode`,
`UnwrapOrResultNode`, `UnwrapOrElseResultNode`, `NotNode`, `AndBoolNode`,
`OrBoolNode`, `XorBoolNode`, `ClonedNode`, `ClonedOptionNode`,
`ClonedResultNode`, `MapErrNode`, `OrElseNode`, `OkResultNode`,
`DiscardOptionNode`, `DispatchNode`, `TeeNode`.

## Codegen impact

Named chain types produce **identical assembly** to the previous closure-based
implementation. Verified via `cargo asm` across all 243 codegen audit functions
— instruction counts match exactly (e.g., 30-step pipeline: 21 instructions,
50-step pipeline: 33 instructions). LLVM inlines through named node types just
as effectively as closures.
