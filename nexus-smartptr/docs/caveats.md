# Caveats

`nexus-smartptr` relies on two implementation details of the Rust
compiler that are not (yet) part of the language specification. Both are
stable in practice on every target we care about, and both are verified
at build time, but it's worth understanding the shape of the assumption
before depending on this crate.

## The fat-pointer layout assumption

A `*const dyn Trait` in Rust is a *fat pointer* consisting of two
machine words:

- A data pointer to the concrete value.
- A metadata pointer (for `dyn Trait`, this points to the vtable; for
  `[T]`, this is the length).

`nexus-smartptr` assumes these are laid out in memory as `(data,
metadata)` in that order. Every rustc release we've tested uses that
ordering on every supported target, because LLVM's struct layout rules
and rustc's implementation strategy have been stable since before 1.0.

**It is not part of the Rust language spec.** The Rust reference does
not currently commit to this layout; in principle, a future rustc
version could reorder the fields. If that ever happens, `nexus-smartptr`
will fail to build (see below) rather than miscompile — no silent
breakage, no UB.

This assumption is borrowed from the [smallbox](https://github.com/andylokandy/smallbox)
crate, which makes the same tradeoff.

## Build-time validation

`nexus-smartptr` ships a `build.rs` that checks the fat-pointer layout
on the host compiler before the library compiles:

```rust,ignore
// Simplified — see build.rs in the crate root
#[repr(C)]
struct FatPtr {
    data: *const u8,
    meta: *const u8,
}

let val = ProbeImpl;
let data_ptr: *const u8 = &raw const val as *const u8;
let trait_ptr: *const dyn Probe = &val as &dyn Probe;

let decomposed: FatPtr = unsafe { ptr::read(ptr::addr_of!(trait_ptr).cast::<FatPtr>()) };

assert!(
    decomposed.data == data_ptr,
    "nexus-smartptr: trait object layout is not (data, vtable). \
     This compiler is not supported."
);
```

The same check is performed for slice pointers (`(data, len)`). If
either check fails, the build aborts with a clear error. You will
never get a successful build of `nexus-smartptr` on a compiler where
its assumptions are wrong.

Because the layout is a *compiler property* (not a target property),
running `build.rs` on the host catches the problem even when
cross-compiling.

## Alignment and padding

Buffers are `#[repr(C, align(8))]` byte arrays. The `define_buffer!`
macro has a compile-time assertion that
`size_of::<Self>() == declared_size`, which rejects any size that isn't
a multiple of 8 (because alignment padding would otherwise grow the
struct).

```rust,ignore
nexus_smartptr::define_buffer!(B512, 512);  // OK
nexus_smartptr::define_buffer!(B12, 12);    // compile error
```

If you need a custom size, pick one that's a multiple of 8.

## Unsized capacity reduction

For `?Sized` targets, one pointer-sized word is reserved inside the
buffer for metadata. On a 64-bit target, `Flat<dyn Trait, B32>` has
**24 bytes** of usable value space, not 32. On a 32-bit target, the
reduction is only 4 bytes, so usable capacity would be 28.

For `Flex`, subtract *another* pointer-sized word for the inline/heap
discriminant. This sometimes surprises people migrating from `Flat` to
`Flex` when values that used to fit inline now fall back to the heap.
The fix is either bumping the buffer size or accepting the fallback.

## No shared ownership

`Flat` and `Flex` are owning pointers. There is no reference counting,
no `Clone` trait, no ability to hand out shared references beyond
normal borrowing. If you need shared ownership, wrap the `Flat`/`Flex`
in an `Rc` or `Arc`:

```rust,ignore
use std::sync::Arc;
use nexus_smartptr::{flat, Flat, B32};

let shared: Arc<Flat<dyn SomeTrait, B32>> = Arc::new(flat!(MyType));
```

This pattern defeats the whole point of inline storage (the `Arc`
allocates), so it's rarely what you want — but it works.

## Miri

`nexus-smartptr` is miri-tested. The unsafe ops (ptr casts, metadata
reconstruction, drop dispatch) pass under the default miri settings
with no leak suppressions needed. If you hit a miri failure when using
this crate in your own code, it's most likely an aliasing or provenance
issue in how you're *holding* the `Flat`/`Flex`, not inside the crate
itself — but please file an issue either way.
