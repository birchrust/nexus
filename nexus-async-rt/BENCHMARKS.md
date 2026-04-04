# nexus-async-rt Benchmarks

## Dispatch Overhead: Async vs Sync

**Question:** What does Rust's async machinery cost compared to manually
polling a handler in a loop? If you hand-rolled your own event dispatch
via nexus-rt's `Handler::run`, how much faster would it be than using
the async executor?

**Answer:** ~2 cycles at p50 for equivalent work. Async dispatch is
effectively free.

### Methodology

All measurements use `rdtsc`/`rdtscp` with `lfence` serialization.
Samples are **batched**: one rdtsc pair wraps 100 iterations, and the
total is divided by 100. This amortizes the ~20 cycle rdtsc floor
to ~0.2 cy per sample, giving sub-cycle resolution.

Each benchmark runs 100K warmup iterations followed by 1M sample
batches. Results are reported as percentiles from sorted samples.

The sync and async paths do the **same trivial work** (counter
increment via `wrapping_add`). The delta is purely framework
dispatch overhead — not user code.

Run with:
```bash
cargo test -p nexus-async-rt --release -- --ignored --nocapture dispatch_latency
```

### Results

All values in **cycles** (rdtsc). Measured on AMD/Intel x86_64.

| Path | p50 | p99 | p999 | Description |
|---|---|---|---|---|
| sync `Box<dyn Handler>` (0 params) | 1 | 1 | 2-4 | Pure dispatch — vtable call, no param resolution |
| sync `Box<dyn Handler>` (1 param) | 4 | 5 | 6-11 | + pointer deref for `ResMut<T>` |
| async poll (IO-woken) | 3 | 5-8 | 8-13 | Task pre-queued by IO driver, executor polls it |
| async poll (self-waking) | 12 | 19-26 | 45-60 | Task re-queues itself via `wake_by_ref` |

### Analysis

**Sync 0 params (1 cy)** is the theoretical floor: a vtable call through
`Box<dyn Handler>` with no resource access. LLVM devirtualizes when
there's a single concrete type, so this is essentially a direct call.
In production with multiple handler types in a mio slab, expect ~3-5 cy
from branch predictor misses.

**Sync 1 param (4 cy)** adds one pointer deref for `ResMut<T>`. This is
the realistic nexus-rt dispatch path — pre-resolved `ResourceId` means
no HashMap lookup, just a single deref from the stored `NonNull<u8>`.

**Async IO-woken (3 cy)** is the production async path: a task was idle,
the IO driver set its `is_queued` flag and pushed its pointer to the
ready queue, then the executor polls it. The 3 cycles cover: Vec
iterate, flag clear, data pointer update on the reusable waker, and
the indirect `poll_fn` call through the task header.

**Async self-waking (12 cy)** includes the cost of `wake_by_ref` inside
the future: TLS read for the ready queue pointer, `is_queued` flag
check + set, and `Vec::push`. This path occurs when a task wakes
itself (e.g., a busy-poll loop). In production, most wakeups come
from the IO driver or timer, not from inside the future.

### What the delta means

The gap between sync dispatch (1-4 cy) and async IO-woken (3 cy) is
**~2 cycles**. At 3.5 GHz that's ~0.6 ns. For any real workload —
WebSocket frame parsing, order book updates, REST response handling —
this is unmeasurable noise.

The async executor does not impose meaningful overhead over hand-rolled
sync dispatch. The benefit of async (composable IO, multiplexed
connections, structured concurrency via spawn/join) comes at
effectively zero dispatch cost.

### What the async executor actually does per poll

For a single IO-woken task, the hot path is:

```
Vec::iterate          — load task pointer (sequential, prefetch-friendly)
clear is_queued       — 1 byte store at offset 16
update waker data     — 1 pointer store (reusable waker, hoisted setup)
call poll_fn          — indirect call through task header vtable
```

No heap allocation. No TLS access (unless the future calls
`wake_by_ref`). No HashMap lookup. No VecDeque index math (double-buffer
Vec swap). The waker vtable and Context are pre-built once per `poll()`
call and reused across all tasks.

### Spawn + free overhead (separate concern)

Task spawn allocates into a nexus-slab byte slab (placement new, O(1))
and task completion frees the slot (freelist push, O(1)). This is a
separate concern from dispatch overhead — spawn/free happens once per
task lifetime, not per poll.

| Operation | p50 | Description |
|---|---|---|
| Slab alloc + free (256B) | 8 cy | nexus-slab byte slab, placement new |
| Spawn + poll + free (0B future) | 26 cy | Full lifecycle for immediate-completion task |
| Spawn + poll + free (64B future) | 28 cy | Includes 64-byte memcpy into slab |
