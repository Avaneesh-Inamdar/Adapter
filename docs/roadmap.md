# Evidence-ranked roadmap

The authoritative delivery plan is [P0–P6](phases.md), covering all 36 requested
items, dependencies and five binary application gates. This page retains the
history and detailed D1–D5 service implementation tracks. A completed slice
does not mean its entire subsystem is complete. The first libc Mac differential
slice is verified; general macOS/Xcode compatibility is not established.

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
- [x] D1 dynamic-loading slice: an unchanged Mach-O host loads classic, chained
  and `MH_BUNDLE` plugins after startup, resolves function/data/string exports,
  observes constructors/destructors, duplicate and `RTLD_NOLOAD` handles,
  `RTLD_LOCAL`/`FIRST` scope, per-thread one-shot errors, and pre-constructor
  rejection of dynamic TLS. See [results](dynamic-loading-results.md) and
  [evidence](reverse-engineering/dynamic-loading-evidence.md). Images are never
  unmapped and the listed scope restrictions remain.
- [x] D2 Mach IPC slice: unchanged Mach-O server and client processes exchange a
  request and send-once reply through a loopback broker, with bootstrap check-in
  and lookup, XNU receive-header inversion, right consumption and documented
  failure codes. See [results](mach-ipc-results.md) and
  [ABI evidence](reverse-engineering/mach-ipc-evidence.md). Complex messages,
  port sets and notifications remain absent; XPC has its own D4 slice.
- [x] D3 launchd-style bootstrap slice: the broker loads job plists, reserves
  `MachServices` names, starts jobs on first lookup or `RunAtLoad`, applies
  environment/log redirection, waits for check-in, respawns after exit and
  replays spawn failures as unknown-service. See [results](launchd-results.md)
  and [evidence](reverse-engineering/launchd-bootstrap-evidence.md). This is an
  Adapter control plane: no launchd PID 1, `launchctl`, `launch_msg`, KeepAlive,
  socket activation or per-user domains.
- [x] D4 XPC slice: unchanged libxpc client/server processes exchange a typed
  dictionary request and reply through the broker, with listener check-in,
  demand spawn, event-handler blocks, containers, synchronous replies, remote
  reply connections, cancellation/interruption and missing-service errors. See
  [results](xpc-results.md) and [evidence](reverse-engineering/xpc-evidence.md).
  The serialization is adapter-defined: there is no real-macOS peer
  interoperability, `xpc_main`/GCD execution, peer validation or sandbox and
  entitlement enforcement.

## OS services and dynamic loading track

macOS applications depend on services below and above the BSD/libc layer.
These are separate implementation tracks, ordered by dependency; no layer is
satisfied by adapters for another. D1-D4 now have implemented slices with
measured consumers; D5 remains planning plus the measured gaps in the import
inventory, not compatibility evidence.

| Stage | Component | Current state | Required for | Acceptance gate |
| --- | --- | --- | --- | --- |
| D1 | Runtime dyld loading: `dlopen`/`dlsym`/`dlclose` of Intel dylibs and bundles | Implemented slice: per-image groups, refcounted handles, `RTLD_GLOBAL`/`LOCAL`/`NOLOAD`/`FIRST`, constructors and per-image finalizers, adapter-only install-name groups. Images are never unmapped; dynamic TLS, `@rpath` arguments, `RTLD_NEXT`/`RTLD_SELF`, `dladdr`, interposition and cross-group sharing are unsupported | Plugin architectures, Xcode's `libIDEApplicationLoader.dylib` path, many complex CLIs | Startup consumer met in-process over classic, chained and bundle plugins. Remaining: a pinned real Intel plugin consumer, true unload, dynamic TLS and cyclic initialization |
| D2 | Mach IPC: ports, send/receive rights, messages, bootstrap namespace | Implemented slice: per-worker name spaces, broker-routed simple messages, send/send-once consumption, receive-header inversion, timeouts and LARGE retry, bootstrap check-in/lookup and lifecycle errors. No complex messages, port sets, notifications, introspection or launchd-managed check-in | `launchd`, XPC, parts of CF/Foundation and crash reporting | Met by two unchanged Mach-O processes over a loopback broker plus failure-semantics probes. Remaining: complex messages, port descriptors and a pinned real Intel Mach client |
| D3 | `launchd`/bootstrap: job registration, spawn, on-demand activation, service namespace | Implemented slice: broker-loadable job plists with the documented subset of keys, reserved `MachServices`, lookup- and `RunAtLoad`-triggered spawn through `adapter-run`, environment/log redirection, 15-second check-in wait, one respawn and unknown-service on failure. No launchd process, `launchctl`/`launch_msg`, KeepAlive, throttling, supervision, socket/event activation, per-user domains or pre-created receive rights | XPC services, background tools, Xcode workers | Met by a broker-started, unchanged Mach-O job reached by the unchanged client, including respawn, RunAtLoad, spawn failure and plist rejection. Remaining: KeepAlive/supervision, socket activation, privileged/per-user namespaces and a pinned real launchd consumer |
| D4 | XPC: connections, serialization, peer validation | Implemented slice: typed object registry (null/bool/int64/uint64/double/string/data/array/dictionary), connections over broker links, listener check-in through bootstrap, event-handler blocks on one dispatcher thread, async/sync sends, reply dictionaries, cancel/interruption and demand-spawned listeners. Adapter-defined wire format; no Apple interoperability, `xpc_main`/GCD scheduling, date/UUID/fd/shmem/endpoint/rich-error objects, peer code-signing requirements, entitlements or sandbox | Xcode subsystems and modern app services | Met by unchanged libxpc client/server processes over the broker plus failure probes; sandbox and entitlement limits are explicit. Remaining: a real macOS XPC peer or measured app service, richer object types, peer validation and reference-Mac ordering |
| D5 | Xcode service consumers: SourceKit, CoreSimulator, `xcodebuild`, plugins | Xcode is ARM64; Objective-C and GUI prerequisites are also missing | Xcode | Measured service call from a real Intel target after D1-D4 and Objective-C/Foundation |

The broker daemon (mapping Mach ports to loopback TCP with a token) is an
engineering choice, not a measured Apple design. `launchd` is PID 1 on Darwin,
and Adapter has no process personality or `fork`/`exec`, so D3 is an
adapter-defined job control plane rather than a thin plist translation: the
broker spawns an unmodified Mach-O guest with `adapter-run` when a registered
service is looked up. D1 was the first track because it is closed-loop, testable
without IPC services, and a prerequisite for loading Xcode's own
application-loader bundles. Its in-process consumer now exists; a real pinned
plugin target and the unsupported load semantics remain before that Xcode path
can be attempted. D2 followed because launchd and XPC are expressed in Mach
messages; the cross-process consumer and the D3 job path now exist, but complex
messages and a pinned real Mach client remain open. D4 then implemented libxpc's
public object and connection model over broker links; an unchanged client/server
pair exercises typed dictionaries, but Apple's wire format, richer object types,
peer validation and sandbox/entitlement enforcement remain open. The broker is
deliberately a routing and job-control service for trusted local workers, not a
kernel personality or security boundary.

## Next acceptance gates

1. **Expand binary and API evidence.** Add the next pinned Intel command-line
   target and selected upstream jq/ripgrep cases; identify actual missing APIs
   before extending them. Extend the hosted-Mac differential job beyond the
   first 4,096 libc cases. Extend formats, timezones, metadata fidelity and numerical edge
   behavior from concrete failures. Gate: reproducible output/error assertions,
   provenance, explicit unsupported cases and no regression in existing tests.
2. **Thread and loader lifecycle.** Condition variables and C++ TLV destruction
   are verified in-process. Runtime dyld loading now has a consumer for classic,
   chained and bundle plugins. Remaining: pthread cancellation (including cleanup
   handlers), process-shared synchronization objects, true unload, TLS-bearing
   plugins, interposition/`RTLD_NEXT` and cyclic initialization. Gate: measured
   consumers for each; cancellation and load/unload lifetime tests, not just
   create/destroy. Current cycles and unsupported exports remain explicit
   pre-constructor failures.
3. **Mach IPC depth, launchd bootstrap and XPC interoperability.** The
   two-process message path is verified for simple messages, the broker starts
   registered jobs on demand or at load, and an unchanged libxpc pair exchanges
   typed dictionaries. Remaining D2 work: complex messages and port descriptors,
   out-of-line memory, port sets and notifications, introspection calls, and a
   pinned real Intel Mach client. Remaining D3 work: launchd `KeepAlive` and
   supervision, socket/event activation, pre-created receive rights and
   privileged/per-user domains. Remaining D4 work: Apple-compatible
   serialization or a measured app service, date/UUID/fd/shmem/endpoint and
   rich-error objects, GCD queue semantics, peer requirements, and explicit
   sandbox/entitlement behavior. Gate: a pinned real launchd consumer and a real
   macOS XPC peer, plus queue fairness, right-transfer, failure-order and
   broker-loss behavior compared with a reference Mac.
4. **Extend the Apple Objective-C runtime.** The raw-isa MRC slice executes
   actual metadata, absolute and relative method lists, startup categories,
   inheritance, initialization, mixed-register/stack dispatch and explicit
   runtime-owned ownership in classic, chained and relative consumers. Next:
   ARC/weak with native Mac lifetime comparisons and stronger ownership
   coverage. Protocol/property introspection now has compiled consumers and a
   same-artifact Mac comparison. See [contract v3](abi/objc-abi.md). Exceptions and
   dynamic registration remain separate gates.
5. **Foundation and broader CF.** Follow actual imported types/methods. Start
   with constant CF strings, ownership and collections before bridging. Gate:
   targeted consumers with verified object layouts and lifetime behavior.
6. **GUI consumer.** Clipboard Viewer has an Intel slice with 55 undefined
   symbols/five direct dependencies. Objective-C, Foundation, AppKit event loops
   and resources are prerequisites; Pixie also needs private graphics APIs.
   Gate: observed window creation, input/events, drawing and orderly teardown.
7. **ARM64 translation.** Treat CPU translation, ABI/TLS/exception integration
   as a distinct architecture. Gate: controlled ARM64 machine-code probes then
   a pinned real CLI target with matched outputs. Native x64 mapping alone
   cannot execute the sampled ARM64 Xcode/compiler images.
8. **Xcode and wider OS services.** Continue DVTFoundation/DVTUserInterfaceKit/
   IDEKit analysis and dynamic dependency discovery. Gate: a measured service/
   framework consumer before claiming launcher or IDE support. Track D1-D5
   above; Mach services, `launchd` supervision, XPC interoperability, security,
   networking, developer tools and GUI integration remain major work.

Keep all existing dylib, TLS, parser, archive, libc and jq tests as regression
requirements. README percentages count source-registered adapters against the
known Intel import inventory. The working-feature and remaining-work tables
track behavior that those counts do not measure.
