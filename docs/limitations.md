# Limitations and risk register

| Risk | Current status / mitigation | Next evidence needed |
|---|---|---|
| ARM64 Xcode/toolchain on x64 host | All sampled core Xcode/CLT images ARM64 only; execution rejected | A separately scoped instruction translator or Intel artifact |
| Apple Objective-C ABI | Bounded raw-isa MRC startup runtime with absolute/relative method lists and startup categories verified with real compiled consumers; a hosted Intel Mac matched all 4,096 same-binary differential probes | Protocols/properties, ARC/weak, dynamic registration, exceptions and real-library lifecycle behavior; see [contract v2](abi/objc-abi.md) |
| Framework surface and GUI | Foundation/AppKit/SwiftUI absent | Minimal real app requirements and reference execution |
| Grand Central Dispatch | Queues, barriers, groups, once/after/apply, block objects, timer and user-data sources, `__block` promotion and the main queue run with Mach-O consumers ([results](dispatch-results.md)) | Descriptor, process, signal, Mach and vnode sources, `dispatch_data`/`dispatch_io`, workloops, QoS scheduling and a reference-Mac comparison |
| Mach VM | `mach_vm_allocate`/`deallocate`/`protect` and the `vm_*` spellings run over the host address space with Darwin return codes and a verified consumer; only adapter-created ranges are releasable | Region queries, sharing, sub-range release, inheritance and ledger accounting; no reference-Mac comparison yet |
| Darwin system calls / Mach services | No syscall interception or kernel personality. A D2 slice provides per-worker Mach namespaces, a loopback broker, simple messages with send/send-once rights, timeouts and a bootstrap service table | Complex messages, port sets, notifications and a pinned real Mach client; syscall probes |
| Service management and XPC | D3 lets the broker start launchd-style jobs on lookup or `RunAtLoad`, with a documented plist subset, environment/log redirection and check-in wait. D4 adds typed dictionary XPC between Adapter workers with listener check-in, demand spawn, blocks on a dispatcher thread, replies and invalidation. There is no launchd process, `launchctl`/`launch_msg`, KeepAlive, supervision, throttling, socket/event activation or per-user domain. XPC has no Apple wire format, `xpc_main`, GCD semantics, richer object types, peer validation or sandbox/entitlement enforcement, and `fork`/`exec` remain absent | KeepAlive/supervision, socket activation, pre-created receive rights, a real macOS XPC peer, and explicit sandbox/entitlement policy |
| TLS and pthread semantics | Basic TLV templates, threads, keys, once, errno, stack-size attributes, naming, normal/recursive mutexes, condition variables and C++ thread-exit destructors verified; a hosted Intel Mac matched all 1,024 same-binary threading probes | Cancellation, process-shared condition variables and TLV destructor ordering against `atexit` |
| Dynamic libraries | Local Intel graph execution, reexports and runtime `dlopen`/`dlclose` of classic/chained/bundle plugins verified; cycles rejected; close runs finalizers but never unmaps; no dynamic TLS, cross-group sharing, `RTLD_NEXT` or interposition | True unload, TLS-bearing and cyclic plugins, symbol interposition and a real plugin consumer |
| Exceptions/unwind/signals | No Darwin unwinder. Signals are synchronous only: dispositions, masks, pending delivery and default termination are verified for the calling thread; no asynchronous delivery, per-thread masks, siginfo or fault mapping ([results](signal-results.md)) | Cross-image C++/ObjC throws, Windows SEH interaction |
| Modern dyld variants | Chained formats restricted; no weak/flat binding | Real target-specific encodings + negative regression corpus |
| ABI differences | SysV calls plus measured printf-family varargs and x87 argument adapters | General aggregates/vectors, wide/positional/long-double formatting, exception/unwind interaction |
| Runtime semantic equivalence | First 4,096-case libc same-artifact hosted-Mac comparison; jq/ripgrep and other subsystem tests remain Windows-side | Expand reference coverage beyond eight C library families; see [harness](differential-testing.md) |
| Untrusted code / parsing | Bounds/resource caps; worker timeout; not a security sandbox | Dedicated fuzzing and hardened execution isolation |
| Apple authentication | Embedded content checksums verified where reported; signer trust unverified | Independent trusted Apple signature/certificate validation |
| REA limitations | Portable inventories work; Ghidra is installed but the win32 provider accepts PE only; Hopper needs macOS and IDA is unconfigured | A supported engine/host; no invented decompiler findings |
| Guest stack reserve | Worker now reserves 8 MiB like Darwin's main thread; MSVC default 1 MiB overflowed jq recursion | Deep-recursion consumers beyond jq's suite; secondary-thread stack equivalence |
| Local execution policy | An earlier milestone's launches were denied by Windows Application Control (4551); the current full run passed without policy changes | Record any future policy blocks; no bypass performed |

Unsupported executable formats include Mach-O32, swapped-endian thin files,
ARM execution, non-PIE files, LC_UNIXTHREAD entry, encrypted/protected images,
section relocations and shared-cache/preinitialized TLVs. Objective-C admission is
limited to the metadata and message ABI in [contract v1](abi/objc-abi.md).
Thin C++ TLV initialization functions execute as guest code through the adapter's
TLV lookup; shared-cache TLV layouts remain unsupported. Code signatures
are reported by command location, not verified or enforced. Unknown required
load commands are rejected for execution.

Only the jq/ripgrep-oriented filesystem, locale, FILE/stat, threading and libc
slice is implemented.
C/POSIX locale is supported; Unicode/system locales are not. stdio uses separate
guest/host objects; full Darwin buffering/private locks and cookie streams are
absent. printf-family positional, wide and 80-bit long-double formatting is
unsupported; field width/precision is capped at 1 MiB. Host floating/pointer
format details, math errno/fenv, complete time formats/timezone behavior and
POSIX filesystem semantics remain incomplete. stat uid/gid are zero, permissions
are synthesized and ctime uses modification time. Directory descriptors are
unsupported. Direct environment-array mutation is unverified.

Condition variables exist in-process, but a condattr marked process-shared does
not enable cross-process waiting because the host object is process-local.
Networking, resource lookup APIs, fork/exec, security services, Metal and AppKit
event loops are absent. Mach ports now have a D2 slice: per-worker namespaces,
send/send-once and receive rights, simple messages with the Darwin receive
header inversion, timeouts and a loopback broker that owns service names. This
broker routes messages between trusted local workers; it is not a Mach kernel,
adds no memory protection or kernel-enforced right transfer, rejects complex
messages, out-of-line memory, port sets and notifications. The broker can also
load launchd-style job plists and spawn registered guests on service lookup or
`RunAtLoad`, but it is not launchd: there is no PID 1, `launchctl`, KeepAlive,
supervision, throttling, socket activation, privileged/per-user namespace or
pre-created receive right, and running jobs are not terminated when the broker
exits. XPC connects only Adapter workers: payloads use an adapter-defined
format, so a real macOS process cannot be a peer. There is no `xpc_main` or GCD
execution, and dates, UUIDs, file descriptors, shared memory, endpoints, rich
errors, peer requirements and sandbox/entitlement checks are absent. The broker
does not authenticate or sandbox either peer. Mach ports are not substitutes
for POSIX sockets or CRT process APIs. Signal handlers and
alternate stacks are recorded but never invoked; pause blocks. pthread stack-size attributes are
honored, but guard size, scheduling and cancellation are not. ripgrep's
preprocessor path therefore fails cleanly with exit 2 instead of starting a
child process.
Xcode's dynamically loaded application libraries make its five direct launcher
dependencies a lower bound, not its complete dependency closure.

Darwin errno storage and common error translation exist; the mapping is incomplete.
Unjoined guest threads are waited for before unmapping rather than terminated
as Darwin exit would do. Nonterminating workers are stopped by the parent
timeout. pthread_t tokens do not implement Apple's internal structure, cleanup
macros or cancellation. Once controls support the public static initializer
contract, not inspection/modification of private opaque state. Apple Clang's
`__tlv_atexit` path runs C++ `thread_local` destructors LIFO at worker exit and at
main-session teardown; destructors that register additional entries are drained
in the same pass. The `__cxa_thread_atexit` spelling, exception-driven
unwinding, pthread cancellation cleanup and Darwin's exact ordering between
key destructors and TLS destructors are not established. XSAVE state over 16 KiB
causes a controlled rejection for programs using TLV templates.

Runtime `dlopen` maps classic, chained and MH_BUNDLE Intel images and runs their
constructors. `dlclose` drains that group's `__cxa_atexit` callbacks and image
finalizers but deliberately never unmaps memory, because retained guest pointers
are not tracked; `RTLD_NODELETE` therefore has no distinct effect. Images with
dynamic TLS are rejected before constructors, `@rpath`/`@loader_path`/
`@executable_path` arguments are not expanded, and loading a dylib that is
already a dependency of another active group is rejected rather than shared.
`RTLD_NEXT`, `RTLD_SELF`, `dladdr`, symbol interposition, cyclic initialization
and ObjC class registration are absent. Handles are adapter-defined process-local
values, not Darwin dyld pointers. `RTLD_LAZY` and `RTLD_NOW` produce the same
eager binding because no lazy resolver exists.

The archive tools create fresh output directories, copy only chosen regular
files, record symlink targets without creating links, bound chunk sizes, and
verify stored payload checksums. They support the observed XAR/PBZX/odc
encodings, not arbitrary Apple installer formats. 7-Zip reported HFS image tail
data warnings. Its XAR PackageInfo/Distribution extraction produced payload
aliases; those copies were discarded as evidence, and the package was decoded
directly from its XAR TOC with checksum validation instead.
