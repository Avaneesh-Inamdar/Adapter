# Thread lifecycle milestone — 2026-10-21

Two previously missing lifecycle pieces now run in-process through unchanged
Mach-O consumers: pthread condition variables and C++ thread-local destructors.
Both are verified on the Intel guest ABI with classic and chained fixups. No
macOS reference run exists, and no recompiled Windows substitute is involved.

## Implementation

- Darwin LP64 condition layouts: 48-byte `pthread_cond_t`, 16-byte
  `pthread_condattr_t`, and the `_PTHREAD_COND_SIG_init`/`0x3CB0B1BB` static
  initializer contract from the pinned Xcode 27 headers. See
  [ABI evidence](reverse-engineering/thread-lifecycle-evidence.md).
- `pthread_cond_init/destroy/wait/timedwait/signal/broadcast` over host
  `CONDITION_VARIABLE`s. Wait and timedwait validate the paired mutex and
  release/reacquire it through `SleepConditionVariableSRW`; timeout conversion
  uses the absolute `CLOCK_REALTIME` deadline and returns Darwin `ETIMEDOUT`.
- `pthread_condattr_init/destroy/getpshared/setpshared`, including the default
  private value `2`, accepted shared value `1`, and `EINVAL` for other values.
  The attribute object is validated and reported; no process-shared object is
  created.
- C++ TLV destruction through the observed Apple Clang contract:
  `__tlv_atexit(destructor, object, __dso_handle)` registers per-thread
  destructors. Cleanup drains in LIFO order, including destructors registered
  during cleanup. Worker exit runs them on the guest thread; the main thread's
  registrations run at session teardown. `___dso_handle` resolves to a stable
  adapter symbol but does not filter by image.
- Static and dynamically initialized condition variables are both accepted.
  Destroy clears the signature and a second destroy, signal or wait on a stale
  object returns `EINVAL`.

## Consumers and assertions

`tests/fixtures/condvars.c` is a cross-compiled Mach-O using raw Darwin layouts
and `PTHREAD_COND_INITIALIZER`, not the adapter's internal types. It checks:

- Four workers increment a guarded counter, wait on a broadcast and broadcast
  completion; results are joined in creation order.
- A signal/handoff pair wakes one waiter.
- Default/set/get pshared values and rejection of value 3.
- Runtime init/destroy with double-destroy `EINVAL`; stale-object signal/wait
  `EINVAL`.
- Timedwait returns `ETIMEDOUT` for a +50 ms deadline and for an already-past
  deadline; the mutex is reacquired (trylock reports `EBUSY`); invalid
  nanoseconds return `EINVAL`.

Expected output: `conds:wait, signal, broadcast and timedwait passed\n`.

`tests/fixtures/cxx_tls.cpp` declares a `thread_local` object with a nontrivial
destructor. Three workers each construct it on first use, read its initialized
value, and write `C`; destruction writes `T`. After joins, the main thread uses
its own instance and writes `M`, then prints `cxx:done`. Main-thread destruction
writes a final `T` during session teardown. The test asserts exactly three `C`
and three `T` before `Mcxx:done\n`, and one `T` after it.

## Verification

The current run passed **110 distinct tests: 24 Rust, nine archive and 77
integration.** All 77 integration tests also pass against the release build.
The two new fixtures run in both classic and chained forms; the existing jq and
ripgrep suites remain in the same run. `cargo fmt`, Clippy with `-D warnings`,
and the native `clang-format` check are clean.

The condition-variable symbols are not part of the jq/ripgrep import inventory,
so the generated API coverage percentage is unchanged. Coverage counts adapter
availability against inspected imports, not implemented API breadth.

## Boundaries

- In-process use only. `PTHREAD_PROCESS_SHARED` values are stored but no shared
  memory or cross-process wakeup exists.
- No cancellation, cleanup handlers, robustness, errorcheck/adaptive mutexes,
  `pthread_cond_signal_thread_np` or relative timedwait.
- Condition storage after `__sig` is an adapter choice, not Darwin's private
  layout. Only public static-initializer semantics are claimed.
- TLV destructor ordering relative to key destructors, `atexit` callbacks and
  image finalizers is defined by the adapter, not verified against macOS.
  `__cxa_thread_atexit`, exception-driven cleanup and C++ TLS in shared-cache
  images remain unsupported.
- No reference-Mac execution or same-artifact output comparison exists.

## Reproduction

```powershell
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime.RuntimeTests.test_condition_variables
python -m unittest tests.integration.test_runtime.RuntimeTests.test_cxx_thread_local_destructors
python tools/verify.py
```

The pinned header extracts and their provenance records are under
`target/artifacts/xcode27-sdk/`; regenerating them requires the supplied
`files/Xcode_27.xip` and the verified inventory under
`target/artifacts/xcode27-inspection/`.
