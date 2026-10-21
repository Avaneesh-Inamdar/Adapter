# Intel dylib and thread runtime iteration

Historical milestone report. The later jq/locale/stdio blocker was resolved in
[the jq/libc iteration](jq-libc-results.md); test counts below describe this
earlier milestone rather than the latest repository state.

Completed Intel dylib execution, then progressed into Darwin TLV/TLS, pthread
lifecycle, once initialization, thread-specific keys, destructors and errno.
All test inputs remain actual cross-linked Intel Mach-O files loaded unchanged
by native Windows executables. No Apple application compatibility is claimed.

## Implemented and demonstrated

- Canonical-path deduplication and execution of transitive MH_DYLIB graphs.
- Public export tries and external-nlist fallback through object 0.40.0;
  two-level import ordinals, bounded ordinary symbol addends, and reexports.
- Whole-graph import checking before mapping; all-image fixups/callback
  validation before constructors; dependency-first initialization.
- Shared globals, reverse normal-return teardown, and DSO-filtered
  __cxa_atexit/__cxa_finalize plus atexit registration.
- Lazy per-thread/per-image TLV templates: nonzero, zero-fill, relocated pointer
  content and 64/128-byte alignment. Imported TLS descriptors work across dylibs.
- A TLV assembly bridge that preserves caller state with XSAVE/XRSTOR or
  FXSAVE/FXRSTOR, including SSE/AVX state. On this host the AVX branch ran and
  printed the success message without the explicit AVX-unavailable suffix.
- pthread create/join/self/equal; once initialization; create/delete/get/set
  thread-specific keys with destructor rearming; independent errno pointers.

The three-image fixture produces this exact output for classic and chained
fixups, including normal C destructor registration:

```text
leaf:init
middle:init
main:init
dylib:passed
main:fini
middle:fini
leaf:fini
```

The TLS graph prints `tls:8 threads and 2 images passed`. The thread-state
consumer prints `threads:once, keys, destructors and errno passed`. It verifies
once initialization runs once across eight concurrent callers, keys remain
thread-local, destructors can rearm their values, and pthread errors preserve
errno. A five-image graph imports the same `_collision` name from two different
providers and checks distinct return values (11 and 29).

## Verification

`python tools/verify.py` ran formatting, Clippy with warnings denied, Rust tests,
debug build, LLVM fixture generation, archive tests, debug integration, release
build and release integration. The final run passed all nine stages:

| Group | Passed | Failed |
|---|---:|---:|
| Rust tests | 17 | 0 |
| Archive tests | 9 | 0 |
| Integration, debug | 47 | 0 |
| Same integration cases, release | 47 | 0 |

73 distinct automated tests passed. After strengthening the namespace fixture
to use five images and two actual undefined `_collision` imports, that case was
rerun successfully against both profiles. Its separate logs are retained in
[namespace-regression.json](test-results/namespace-regression.json).
`python tools/check_apple_corpus.py` also verified the identities and successfully
reinspected all 43 selected Apple images with the extended parser; none ran.
Exact pipeline commands/logs are in [latest.json](test-results/latest.json).

An earlier release artifact was blocked by Windows Application Control:
WinError 4551, CodeIntegrity events 3033/3077. This was recorded, not treated as
a test pass. The final normal build following export-lookup changes was allowed
and passed the release suite; no signing/security policy was changed. The cause
of the different policy decision is not established. Initial event evidence remains
in [code-integrity.json](test-results/code-integrity.json).

## Findings and limits

LLD emits C destructors as __cxa_atexit registrations, which required an actual
callback registry rather than only __mod_term_func handling. A pure reexport
dylib can have an empty explicit export trie; object 0.40.0 attempted to decode
an empty terminal, so the inspector now treats that case as an empty public
namespace while retaining dependency reexports. The hidden-symbol regression
ensures an existing private nlist name cannot bypass the export trie.

Apple's [TLV layout reference](https://github.com/apple-oss-distributions/dyld/blob/main/libdyld/ThreadLocalVariables.h)
and [x86-64 helper](https://github.com/apple-oss-distributions/dyld/blob/main/libdyld/threadLocalHelpers.s)
establish the descriptor and special register-preservation contract. Public
[pthread types](https://github.com/apple-oss-distributions/libpthread/blob/main/include/sys/_pthread/_pthread_types.h),
[pthread APIs](https://github.com/apple-oss-distributions/libpthread/blob/main/include/pthread/pthread.h),
[Darwin errno values](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/errno.h)
and [destructor-pass limit](https://github.com/apple-oss-distributions/Libc/blob/main/include/limits.h)
ground the interface declarations. Tests demonstrate the implemented subset;
reference-Mac equivalence remains unverified.

Cycles, weak coalescing, symbol resolvers, shared-cache TLVs, cancellation,
process-shared synchronization, runtime dlopen/dlclose, Foundation and GUI
remain absent. Later work added C++ TLV destruction, in-process condition
variables and stack-size attributes; see
[the thread lifecycle iteration](thread-lifecycle-results.md). Unjoined owned
threads are waited for before unmapping, which differs from Darwin exit behavior. Xcode 27 remains ARM64-only in the sampled launcher/tools.

jq 1.8.2 now passes the implemented TLS preparation and chained-fixup stages,
then static binding rejects `__DefaultRuneLocale`. It was not executed. The
next concrete work is a measured Darwin libc data/locale/stdio ABI slice,
including the remaining imported data structures rather than host-CRT aliases.

The optional raw XIP member reader also now labels filesystem placeholders.
The supplied SDK limits.h data fork starts with NULLcanary, so it was not used
as header evidence; raw CPIO extraction does not reconstruct compressed
filesystem attributes. Upstream primary headers supplied that interface fact.

Code changes: inspector/export metadata; loader graph/link/TLS planning; runtime
multi-image execution; new Darwin callbacks/TLV/threads/thread-state modules;
fixture builders and C/assembly consumers; integration/archive regressions;
verification/reporting tools; architecture/support/limitations/roadmap docs.
The full repository additions remain listed in [file-changes](file-changes.md).
