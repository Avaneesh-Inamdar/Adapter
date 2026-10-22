# Grand Central Dispatch milestone — 2026-10-22

Unchanged Mach-O consumers now create queues, submit blocks and functions,
group and wait on work, run timers and user-data sources, and drive the main
queue through Adapter. The implementation is in
[src/darwin/dispatch.rs](../src/darwin/dispatch.rs) with block objects and
sources in [dispatch/](../src/darwin/dispatch/), the Block runtime in
[blocks.rs](../src/darwin/blocks.rs) and pthread QoS in
[qos.rs](../src/darwin/qos.rs).

## What the adapter does

- **Queues.** One registry holds every queue. A single scheduling pass decides
  which queued items may start: serial queues run one item at a time in
  submission order, concurrent queues run items in parallel, a barrier waits for
  earlier items and holds back later ones, a suspended or inactive queue starts
  nothing, and a queue with a target shares that target's serialisation.
  Admitted `dispatch_async` items run on short-lived host threads (at most 64 at
  once) that are set up like guest pthreads, so blocks may use `errno`,
  `pthread` keys and thread-locals.
- **Synchronous submission.** `dispatch_sync`, `dispatch_barrier_sync` and the
  `*_and_wait` forms run the block on the calling thread once the queue admits
  it, and `dispatch_get_current_queue` reports the queue. Re-entering a serial
  queue the thread already owns traps with exit status 132, as libdispatch does.
- **Root and main queues.** `dispatch_get_global_queue` accepts the legacy
  priorities and the five QoS classes and refuses non-zero flags. The main queue
  (`_dispatch_main_q`) is drained only by `dispatch_main`, including blocks sent
  with `dispatch_sync` from other threads.
- **Groups, once, time.** `dispatch_group_*` (enter/leave/wait/notify/async),
  `dispatch_once`/`_f`, `dispatch_after`/`_f`, `dispatch_apply`/`_f` and
  `dispatch_walltime`. A wall-clock deadline is converted to the monotonic clock
  the semaphore adapter uses.
- **Objects.** `dispatch_retain`/`release` with deferred reclamation (a queue
  lives while it has queued or running work), `suspend`/`resume`/`activate`,
  initially-inactive queues, context and finalizer, queue-specific data, target
  queues and assertions.
- **Block objects.** `dispatch_block_create`, `perform`, `wait`, `notify`,
  `cancel` and `testcancel` (also through `dispatch_wait`, `dispatch_notify` and
  `dispatch_cancel`). The wrapper is a real heap block whose invoke function is
  host code.
- **Sources.** Timer and `DATA_ADD`/`DATA_OR`/`DATA_REPLACE` sources, created
  suspended, with registration, event and cancel handlers delivered on the
  target queue, coalesced events and timer fire counts.
- **Block runtime.** `_Block_object_assign` and `_Block_object_dispose` so
  `__block` variables are promoted to the heap with reference counts and
  captured blocks and Objective-C objects are retained.
- **pthread QoS.** Classes and relative priorities are recorded per thread and
  per attribute and reported back.

## Consumers

| Fixture | Asserts |
| --- | --- |
| [dispatch.c](../tests/fixtures/dispatch.c) | labels and root queues; 100 serial items in order; sync sees its queue; two concurrent blocks that each wait for the other; barrier ordering; group wait timeout and notify; `dispatch_once` under contention; `after` ordering; `apply` over 64 indices; two queues sharing a serial target never overlapping; suspend/resume; `__block` promotion and a captured block; queue-specific data; finalizers; inactive queues; `async_and_wait`; wall-clock deadlines; QoS class; main queue and sync from a worker |
| [dispatch_source.c](../tests/fixtures/dispatch_source.c) | block wait/notify/cancel/perform and invalid flags; `DATA_ADD` coalescing, registration and cancel handlers, silence after cancel; `DATA_OR` and `DATA_REPLACE`; a repeating timer that stops on cancel and a one-shot; a refused source type |
| [dispatch_deadlock.c](../tests/fixtures/dispatch_deadlock.c) | sync onto an owned serial queue traps (status 132) |
| [qos.c](../tests/fixtures/qos.c) | main-thread class, per-thread isolation, attribute round trip, invalid classes and priorities, overrides |

All but the trap test run in classic and chained form, and 15 repeated runs of
the two large consumers produced no failure.

## Boundaries

- Sources of type read, write, vnode, proc, signal, Mach send/receive and
  memory pressure cannot be created (`dispatch_source_create` returns NULL);
  their type symbols exist so programs link. They need kernel event sources.
- No `dispatch_data`, `dispatch_io`, `dispatch_read`/`write` or workloops.
- QoS is recorded, not scheduled: Windows thread priorities are untouched, and
  `DISPATCH_BLOCK_BARRIER` does not make a later `dispatch_async` a barrier.
- `dispatch_apply` onto a serial queue runs the iterations in order on the
  caller without waiting for earlier queued work. A root or main queue is not
  recorded as a target.
- Items on a root queue and on user queues each use a host thread per item;
  more than 64 blocked workers stall further admissions.
- A guest that exits from a block terminates the process; queued work is
  discarded at normal exit, while items already running finish first.
- The timeout value returned by waits is the same non-zero value the semaphore
  adapter reports; it has not been compared with a Mac.
- No reference-Mac comparison exists for this interface.

## Public-header coverage

The catalog in [progress/public-api.json](progress/public-api.json) lists every
symbol exported by the public libdispatch, libclosure, signal and QoS headers
and the Mach VM routine definitions, from pinned Apple open-source commits. The
chart in the [README](../README.md#api-coverage) and the
[coverage notes](progress/README.md#public-api-surface) count complete and
partial adapters separately; [progress/partial-api.json](progress/partial-api.json)
records why each partial adapter is partial.

## Reproduction

```powershell
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime -k dispatch -k qos
python tools/extract_public_api.py --check
python tools/verify.py
```
