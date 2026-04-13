# Unsafe Code & Soundness Guarantees

This document describes every category of unsafe code in nexus-async-rt,
the invariants that make each one sound, and how we verify those invariants
under miri. This crate is designed for millions of messages per second on
production trading infrastructure — soundness is non-negotiable.

---

## Guiding Principles

1. **Every unsafe block has a SAFETY comment** explaining why it's sound.
2. **Every unsafe pattern has miri coverage** verifying absence of UB.
3. **Known miri limitations are documented and justified** — not silenced.
4. **UnsafeCell is used wherever raw pointers outlive their source borrow** —
   this is the single most important invariant in the crate.

---

## The UnsafeCell Pattern (Critical)

### The Problem

Rust's `&mut T` grants the compiler a guarantee: nothing else can read or
write `T` through any other path. The compiler uses this for optimizations —
caching values in registers, reordering loads/stores, eliminating redundant
reads. This is MORE restrictive than C/C++ pointers.

When we store a raw pointer derived from `&mut self.field` and later take
`&mut self` again, we violate this guarantee. Two mutable access paths to
the same memory exist:

```rust
// Step 1: derive raw pointer from &mut self.field
let ptr: *mut Vec<T> = &mut self.incoming as *mut _;
TLS.set(ptr);

// Step 2: take &mut self (covers ALL fields including incoming)
self.complete_task(event); // &mut self

// Step 3: read/write through the TLS pointer
// ❌ UB — the &mut self at step 2 invalidated ptr's provenance
unsafe { &mut *TLS.get() }.push(item);
```

### The Fix

Wrap the field in `UnsafeCell<T>`. This tells the compiler: "this field
has interior mutability — don't assume exclusive access through `&mut self`."

```rust
struct Executor {
    incoming: UnsafeCell<Vec<T>>,  // compiler won't assume exclusivity
    other_field: u64,              // compiler CAN assume exclusivity
}
```

Now `&mut self` on Executor asserts exclusivity over `other_field` but
NOT over `incoming`. The TLS pointer (derived from `UnsafeCell::get()`)
remains valid across `&mut self` reborrows.

### Where This Applies in nexus-async-rt

| Field | Struct | Why |
|-------|--------|-----|
| `incoming` | `Executor` | TLS raw pointer for waker push, survives `complete_task(&mut self)` |
| `deferred_free` | `Executor` | TLS raw pointer for deferred slot free, same reason |
| `io` | `Runtime` | TLS raw pointer for IO registration, survives `run_loop` `&mut self` |
| `timers` | `Runtime` | TLS raw pointer for timer scheduling, survives `fire_expired` through `&mut self` |

### How to Know if a New Field Needs UnsafeCell

Ask: "Is a raw pointer to this field stored somewhere (TLS, struct field,
closure capture) that outlives the current `&mut self` borrow?" If yes,
wrap in `UnsafeCell`.

The pattern to look for:
1. `&raw mut self.field` or `std::ptr::from_mut(&mut self.field)` — pointer derived
2. Pointer stored in TLS, another struct, or returned to caller
3. Later in the same function (or a called function), `&mut self` is taken

If all three conditions hold, `UnsafeCell` is required.

### Cost

Zero runtime cost. `UnsafeCell` is `#[repr(transparent)]` — same size,
same alignment, same layout. `get()` is a no-op pointer cast. The only
effect is that the compiler skips exclusivity-based optimizations on that
field, which for runtime driver fields accessed a handful of times per
poll cycle is unmeasurable.

---

## Unsafe Code Categories

### 1. Task Header (task.rs) — 106 unsafe blocks

Type-erased task storage. Each task is a `repr(C)` struct with a 64-byte
header containing function pointers (`poll_fn`, `drop_fn`, `free_fn`),
atomic flags, and a refcount. The storage region after the header holds
either the future `F` or the output `T` (union transition on completion).

**Key invariants:**
- Function pointers at offsets 0/8/16 are set at construction and never change
  (except `drop_fn` which is overwritten from `drop_future` to `drop_output`
  on completion)
- `storage_offset` (offset 56) correctly points to the `storage` field
  via `offset_of!` — verified by the compile-time assertion
  `size_of::<Task<()>>() == TASK_HEADER_SIZE`
- Refcount (AtomicU16 at offset 26) is incremented for each waker clone
  and decremented on drop. Exactly one `ref_dec` returns `should_free=true`.
- Union transition: `poll_join` drops `F` in place, writes `T` to the same
  bytes, overwrites `drop_fn`. The old `F` is dead, the new `T` is live.

**Miri coverage:** 28 tests in `miri_task.rs` — spawn, union transitions
for u64/String/Vec/DropCounter, JoinHandle lifecycle (read, detach, abort),
refcount balance, output larger than future, ZST output, multi-task
interleaving, executor drop cleanup.

### 2. Waker Vtable (waker.rs) — 25 unsafe blocks

Four vtable functions (`clone_fn`, `wake_fn`, `wake_by_ref_fn`, `drop_fn`)
that operate on raw task pointers. Each reads/writes atomic fields at fixed
offsets in the task header.

**Key invariants:**
- The waker data pointer IS the task pointer — no indirection
- `clone_fn` increments refcount, `drop_fn` decrements it
- `wake_fn` decrements refcount AND pushes to ready queue
- `wake_by_ref_fn` pushes without decrementing (borrow, not consume)
- `wake_impl` checks `is_completed` (skip dead tasks) and `is_queued`
  (dedup — don't push twice)

**Miri coverage:** 6 tests in `miri_waker.rs` — clone increment, wake
by ref doesn't consume, wake by value consumes, drop after completion
frees slot, wake completed is noop, multiple clones one free.

### 3. Cross-Thread Wake Queue (cross_wake.rs) — 37 unsafe blocks

Vyukov-style intrusive MPSC queue. Each task's `cross_next` field
(AtomicPtr at offset 32) serves as the link pointer. Zero allocation.

**Key invariants:**
- Stub node (heap-allocated AtomicPtr) avoids the empty-queue edge case
- `push` is atomic swap on tail + Release store on prev's next
- `pop` is single-consumer (head is non-atomic, behind UnsafeCell)
- `try_set_queued` CAS prevents double-push of the same task

**Miri coverage:** Partially covered via `miri_task.rs` (waker fires
during poll exercises the ready queue path). The intrusive queue itself
is exercised through the channel miri tests.

**Known miri limitation:** The Vyukov tail CAS uses `Relaxed` ordering.
The CAS provides mutual exclusion (only one thread wins each slot) but
not happens-before. Miri's data race detector reports a false positive
on multi-threaded tests. The actual ordering is provided by the turn
counter protocol, not the CAS. Multi-threaded tests are annotated
`#[cfg_attr(miri, ignore)]`.

### 4. Channels (channel/*.rs) — 125 unsafe blocks

Ring buffers with raw pointer read/write, intrusive waiter lists for
blocked senders/receivers, and Polonius-workaround transmutes.

**Key invariants:**
- Ring buffer: `ManuallyDrop<Vec>` extracts raw pointer, Drop reconstructs
  Vec for deallocation. Elements are dropped in order before Vec dealloc.
- Waiter lists: intrusive doubly-linked list using raw prev/next pointers.
  Pin ensures nodes don't move after registration.
- Polonius transmutes: `std::mem::transmute::<Claim<'_>, Claim<'_>>()` is
  a lifetime-only cast (same type). Required because NLL can't express
  "borrow ends when loop iteration ends."

**Miri coverage:** 15 tests in `miri_channel.rs` — local/spsc/mpsc
send/recv, fill and drain, drop tracking, sender/receiver close.

### 5. Cancellation Token (cancel.rs) — 11 unsafe blocks

Lock-free Treiber stacks for waiter nodes and child token nodes.
Push via CAS, drain-all via atomic swap to null.

**Key invariants:**
- Nodes are heap-allocated via `Box::into_raw`, freed via `Box::from_raw`
  during drain
- Double-check after CAS push catches the register-during-cancel race
- `cancel()` is idempotent — safe to call multiple times
- `Inner::drop` drains remaining nodes (defensive cleanup)

**Miri coverage:** 9 tests in `miri_cancel.rs` — basic lifecycle, 5
waiters + drain, child propagation, register-during-cancel race,
drop without cancel, child drop before parent, waker update on repoll,
many waker changes, cancel before poll.

### 6. Slab Task Allocation (alloc.rs) — 36 unsafe blocks

Type-erased slab allocation via TLS function pointers. `slab_spawn`
constructs a task on the stack, copies it into a slab slot via
`ptr::copy_nonoverlapping`, then forgets the stack copy.

**Key invariants:**
- `ClaimFn` copies exactly `size` bytes from `src` to the slab slot
- `FreeFn` is stored in the task header and knows the slab address
- Size check ensures the task fits in the slab slot
- TLS function pointers are scoped to `block_on` via RAII guard

**Miri coverage:** 4 tests in `miri_alloc.rs` — spawn and free, drop
tracker, claim and spawn, bounded reuse after free. Requires tree borrows
(`-Zmiri-tree-borrows`) due to a known stacked borrows limitation with
`Cell<*mut T>` in the slab's freelist (see nexus-slab docs).

### 7. Net IO (net/*.rs) — 19 unsafe blocks

Mio registration via raw pointers, `waker.data()` extraction for task
identification, and TCP split (aliased `&mut` through raw pointer for
read/write halves).

**Not miri-testable** — requires real TCP/UDP sockets and mio epoll.
Covered by 13 TCP tests + 11 UDP tests in the integration test suite,
plus 517/517 Autobahn WebSocket conformance via nexus-net.

---

## Miri Testing Infrastructure

### Running miri

```bash
# Dedicated test files (recommended — completes in ~15 seconds)
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" \
  cargo +nightly miri test -p nexus-async-rt \
  --test miri_task --test miri_waker --test miri_cancel \
  --test miri_channel --test miri_alloc

# Full inline tests (slower — stress tests reduced via cfg(miri))
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" \
  cargo +nightly miri test -p nexus-async-rt --lib
```

### Why `-Zmiri-ignore-leaks`

Slab backing memory uses `Box::leak` for stable addresses. The leaked
memory is intentional (reclaimed when the slab is dropped, not when
individual slots are freed). Miri flags this as a leak without the flag.

### Why `-Zmiri-tree-borrows`

The slab allocation path stores a raw pointer in TLS, casts through
`*const u8` → `*const Slab<S>` → `&Slab`. Under stacked borrows, the
`Cell<*mut SlotCell>` freelist pointer inside `&self` has its provenance
weakened. Tree borrows (the replacement model) handles `Cell`/`UnsafeCell`
interactions correctly. The code is sound — stacked borrows is overly
conservative for this pattern.

### Test file inventory

| File | Tests | What it covers |
|------|-------|---------------|
| `miri_task.rs` | 28 | Task header, union transitions, JoinHandle, refcount, executor drop |
| `miri_waker.rs` | 6 | Waker vtable clone/wake/drop, refcount lifecycle |
| `miri_cancel.rs` | 9 | Treiber stacks, waiter nodes, waker updates |
| `miri_channel.rs` | 15 | Ring buffers, drop tracking, sender/receiver close |
| `miri_alloc.rs` | 4 | Slab spawn, copy_nonoverlapping, bounded reuse |

### Ignored tests under miri

| Test | Reason |
|------|--------|
| `mpsc::cross_thread_*` (3 tests) | Vyukov Relaxed CAS — miri false positive |
| `tcp_echo`, `tcp_socket_builder` | Requires real TCP sockets |
| `udp_send_recv`, `udp_echo`, `udp_connected` | Requires real UDP sockets |

---

## Adding New Unsafe Code

Before adding any `unsafe` block:

1. **Write the SAFETY comment first.** If you can't articulate why it's
   sound, don't write the code.

2. **Check for the UnsafeCell pattern.** If your code stores a raw pointer
   that outlives `&mut self`, wrap the source field in `UnsafeCell`.

3. **Add a miri test.** Every new unsafe path needs a test in the
   corresponding `miri_*.rs` file. The test should exercise the exact
   code path — miri finds UB on the first iteration, not the millionth.

4. **Run miri locally before pushing:**
   ```bash
   MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" \
     cargo +nightly miri test -p nexus-async-rt --test miri_task
   ```

5. **If miri fails:** Determine if it's a real bug or a known limitation.
   Real bugs: fix the code. Known limitations: document with `#[cfg_attr(miri, ignore)]`
   and a comment explaining WHY it's a false positive. Never silence miri
   without an explanation.
