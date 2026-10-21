# Evidence-ranked roadmap

Each milestone has a concrete consumer and acceptance gate. A completed slice
does not mean its entire subsystem is complete. No reference-Mac equivalence
or general macOS/Xcode compatibility is established.

## Completed milestones

- [x] Bounded Mach-O/FAT inspection and dependency discovery, including the
  selected 43-image Apple corpus.
- [x] Native Intel execution on Windows with explicit SysV entry and adapters.
- [x] Local Intel dylib graphs, two-level exports/reexports, shared data,
  dependency constructors and reverse normal-return teardown.
- [x] Basic TLV/TLS, thread creation/joining, keys/destructors, once and errno.
- [x] Minimal CoreFoundation immutable string and ownership consumer.
- [x] Darwin C/POSIX rune data, separate FILE objects and printf-family varargs.
- [x] Advance through jq's next dependencies: UTF-8 files/stat64, environment,
  UTC/time conversions, memory/string/math adapters and explicit exit flushing.
- [x] Execute unchanged jq 1.8.2 macOS amd64: 13 real-binary tests, plus focused
  libc consumers, passing debug and release. See [results](jq-libc-results.md).
- [x] Pass the pinned upstream jq 1.8.2 `tests/jq.test` suite (550 tests) with its
  module fixtures through the unchanged binary on both build profiles.
- [x] Execute unchanged ripgrep 15.2.0 across 11 CLI workflows, including
  gitignore/hidden rules, JSON, PCRE2, stdin, binary detection, mmap and
  multithreaded search, on both profiles. See [results](ripgrep-results.md).
- [x] pthread condition variables with Darwin layouts, `PTHREAD_COND_INITIALIZER`,
  signal/broadcast/absolute-deadline timedwait and condattr pshared values;
  C++ thread-local construction and `__tlv_atexit` destruction across workers
  and main-session teardown, including cleanup-time re-registration. See
  [results](thread-lifecycle-results.md) and
  [ABI evidence](reverse-engineering/thread-lifecycle-evidence.md).

## OS services and dynamic loading track

macOS applications depend on services below and above the BSD/libc layer.
These are separate implementation tracks, ordered by dependency; no layer is
satisfied by adapters for another. Current state is planning plus the measured
gaps in the import inventory, not compatibility evidence.

| Stage | Component | Current state | Required for | Acceptance gate |
| --- | --- | --- | --- | --- |
| D1 | Runtime dyld loading: `dlopen`/`dlsym`/`dlclose` of Intel dylibs and bundles | Startup graphs, exports, constructors and finalizers work. `dlsym` searches only adapter symbols and ignores its handle; `dlopen`, `dlclose` and `dlerror` are absent | Plugin architectures, Xcode's `libIDEApplicationLoader.dylib` path, many complex CLIs | A Mach-O consumer loads a dylib not linked at startup, resolves and calls its export, observes constructors/destructors, and checks duplicate/missing/symbol errors; then a pinned real target |
| D2 | Mach IPC: ports, send/receive rights, messages, bootstrap namespace | Absent; BSD/POSIX adapters do not emulate it | `launchd`, XPC, parts of CF/Foundation and crash reporting | Controlled two-process send/receive with right and failure semantics; a single-process shim is not sufficient evidence |
| D3 | `launchd`/bootstrap: job registration, spawn, on-demand activation, service namespace | Absent; `fork`/`exec` are explicit failures | XPC services, background tools, Xcode workers | A broker starts a registered job and a measured consumer reaches it; process model and teardown documented |
| D4 | XPC: connections, serialization, peer validation | Absent; depends on D2 and D3 | Xcode subsystems and modern app services | Real libxpc client/server consumer or measured app service; sandbox and entitlement limits explicit |
| D5 | Xcode service consumers: SourceKit, CoreSimulator, `xcodebuild`, plugins | Xcode is ARM64; Objective-C and GUI prerequisites are also missing | Xcode | Measured service call from a real Intel target after D1-D4 and Objective-C/Foundation |

The proposed broker daemon (mapping Mach ports to Windows named pipes or local
RPC) is an engineering hypothesis, not a measured design. Port rights, queue
semantics, bootstrap registration and process teardown need probes before any
broker is claimed. `launchd` is PID 1 on Darwin, and Adapter has no process
personality, `fork` or `exec`, so D3 cannot be a thin plist translation. D1 is
the first track because it is closed-loop, testable without IPC services, and a
prerequisite for loading Xcode's own application-loader bundles.

## Next acceptance gates

1. **Expand binary and API evidence.** Add the next pinned Intel command-line
   target and selected upstream jq/ripgrep cases; identify actual missing APIs
   before extending them. Compare the same artifacts on a reference Mac when one
   is available. Extend formats, timezones, metadata fidelity and numerical edge
   behavior from concrete failures. Gate: reproducible output/error assertions,
   provenance, explicit unsupported cases and no regression in existing tests.
2. **Thread and loader lifecycle.** Condition variables and C++ TLV destruction
   are verified in-process. Remaining: pthread cancellation (including cleanup
   handlers), process-shared synchronization objects, runtime dyld loading
   (D1), and cyclic initialization. Gate: measured consumers for each;
   cancellation and load/unload lifetime tests, not just create/destroy. Current
   cycles and unsupported exports remain explicit pre-constructor failures.
3. **Apple Objective-C runtime experiment.** Start with `objc.m`, then a small
   real Intel target. Register classes/metaclasses/selectors and implement
   dispatch, ownership and initialization from measured layouts. Gate: execute
   actual metadata and method calls, including inheritance/initialization and
   ownership. GNUstep/libobjc2 needs an Apple ABI adaptation experiment.
4. **Foundation and broader CF.** Follow actual imported types/methods. Start
   with constant CF strings, ownership and collections before bridging. Gate:
   targeted consumers with verified object layouts and lifetime behavior.
5. **GUI consumer.** Clipboard Viewer has an Intel slice with 55 undefined
   symbols/five direct dependencies. Objective-C, Foundation, AppKit event loops
   and resources are prerequisites; Pixie also needs private graphics APIs.
   Gate: observed window creation, input/events, drawing and orderly teardown.
6. **ARM64 translation.** Treat CPU translation, ABI/TLS/exception integration
   as a distinct architecture. Gate: controlled ARM64 machine-code probes then
   a pinned real CLI target with matched outputs. Native x64 mapping alone
   cannot execute the sampled ARM64 Xcode/compiler images.
7. **Xcode and wider OS services.** Continue DVTFoundation/DVTUserInterfaceKit/
   IDEKit analysis and dynamic dependency discovery. Gate: a measured service/
   framework consumer before claiming launcher or IDE support. Track D1-D5
   above; Mach services, `launchd`, XPC, security, networking, developer tools
   and GUI integration remain major work.

Keep all existing dylib, TLS, parser, archive, libc and jq tests as regression
requirements. README percentages count source-registered adapters against the
known Intel import inventory. The working-feature and remaining-work tables
track behavior that those counts do not measure.
