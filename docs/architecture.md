# Architecture and decisions

## Measured environment

Audit date: 2026-10-21 (Asia/Calcutta). Workspace `E:\Xcode` was empty and was
not a Git repository; initialized Git without overwriting existing source.
Host: Windows build 10.0.26300, x64. Rust 1.99.0 MSVC target, Python 3.12,
LLVM 23.1.3, MinGW GCC 16.2.0, and CMake 4.4.3 were observed. The implemented
build uses Cargo/MSVC plus Clang C/assembly ABI adapters, not MinGW/CMake. Clang and ld64.lld are installed outside
PATH. The initial work had no reference Mac. A hosted Intel Mac now supplies the
first same-artifact libc comparison; see [differential testing](differential-testing.md).

## Native implementation

Use Rust for bounded parsing and explicit ownership, `object` 0.40.0 for
independent Mach-O validation and symbol decoding, `plist` for XML/binary bundle
metadata, `serde_json` for diagnostics, and `windows-sys` for VM operations.
`Cargo.lock` pins exact resolved versions. The narrow command decoder adds
dyld-specific metadata and defensive checks; it does not replace the entire
maintained parser. Parser alternatives and license/platform findings are in
[component research](component-research.md).

Module boundaries:

| Module | Responsibility |
|---|---|
| `src/inspect` | Thin/fat header selection, commands, segments, sections, symbols, metadata and reports |
| `src/loader` | Bundle discovery, install-name resolution, bounded recursive dependency graph |
| `src/loader/fixups.rs` | Classic dyld rebase, normal bind, eager lazy bind decoding |
| `src/loader/chained.rs` | Chained fixup/import table decoding to a platform-neutral plan |
| `src/loader/link.rs` | Complete link plan, public exports, two-level lookup/reexports, bounded dependency ordering |
| `src/loader/tls.rs` | TLV descriptor/template bounds and alignment validation |
| `src/platform/windows.rs` | VirtualAlloc/VirtualProtect, copying, SG_READ_ONLY final protections, instruction-cache flush, RAII release |
| `src/runtime` | Preflight, import availability, relocation, initialization, entry invocation, worker timeout |
| `src/runtime/dynamic.rs` | Post-startup `dlopen` groups, reference-counted handles, export snapshots, `dlsym` scope, per-image close finalization |
| `src/objc` | Validated raw-isa startup metadata and relative method lists, startup categories, selectors, class initialization, SysV message dispatch and runtime-owned objects/pools |
| `src/runtime/debug.rs` | Optional flushed image/slide/symbol records and LLDB mapped-image breakpoint hook |
| `src/broker` | Loopback Mach transport protocol, endpoint/token handshake, per-connection request dispatch, payload routing, launchd-style job plist loading and on-demand job activation |
| `src/broker/main.rs` | `adapter-broker`: endpoint file creation, service namespace, connection liveness and port retirement |
| `src/darwin` | Rust pthread lifecycle/state, recursive mutexes, condition variables, C++ TLV destruction, dyld image introspection, `dlfcn`, Mach IPC and TLV ABI bridges, Darwin stack probe and exit callbacks |
| `src/darwin/host` | C adapters for libSystem stdio/files/stat/time/locale, signals, process stubs, dispatch, mmap and directory iteration |
| `src/frameworks/corefoundation.rs` | Five opaque immutable-string/ownership adapters |

Execution stages: discover -> inspect graph -> reject unsupported requirements
-> decode/resolve fixups -> reserve/commit/copy -> apply fixups -> validate
initializer pointers -> install final protections -> initialize -> invoke
LC_MAIN -> join owned threads -> finalize -> unmap. A guest `dlopen` repeats
preflight, mapping, relocation and initialization for a new image group without
releasing the startup graph. Errors include their failing stage/symbol.
The graph's `executable_by_current_runtime` flag is only structural eligibility;
`adapter-run --check` additionally validates opcode decoding and bound symbols. A
passed preflight is not an execution or application compatibility claim.

Windows x64 uses a different calling convention from Darwin x64. All guest
entry functions and adapter callbacks explicitly use Rust `extern "sysv64"`;
Windows APIs and host CRT calls use their Windows ABIs. The running fixtures
verify integer argument passing across this boundary and internal guest float
and stack arguments. A native C/assembly layer captures SysV printf-family
varargs into a 24-byte guest va_list and translates each conversion into a typed
Windows CRT call. LP64 long is explicitly widened; guest va_list objects are
never passed to the CRT. General aggregate returns remain unsupported.
Rust documents this ABI in its [external-block reference](https://doc.rust-lang.org/reference/items/external-blocks.html).

The worker reserves a bounded VM span, leaves gaps inaccessible, commits
segments writable for relocation, then applies segment protections before any
guest call. Writable-executable and overlapping segment pages are rejected.
Memory release is tied to ownership. These mechanisms follow the Windows
[VirtualAlloc contract](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc).
The worker has a timeout and crash boundary, not a permission sandbox.

## Loader scope

Only generic Intel x86-64 PIE executables with LC_MAIN execute. Input inspection
also supports little-endian arm64 Mach-O and big-endian FAT/FAT64 containers,
preferring x86-64. Maximum input is 128 MiB, mapping span 256 MiB, graph 256
images, symbols/fixups one million, public exports 100000, and load commands
16384. Prepared file data plus VM spans are capped at 512 MiB in aggregate.

Classic pointer rebases and two-level pointer binds are supported; lazy slots
are eagerly replaced. Special ordinals, weak coalescing, weak imports, nonzero
adapter addends, chained compressed symbols, multi-start pages, high8 rebases,
and other pointer formats are explicit errors. Chained pointer formats 2 and 6
and import formats 1 and 2 have decoders; current linked fixtures exercise the
format emitted by the installed linker. All other formats remain unverified or
unsupported. Metadata definitions are grounded in Apple's
[Mach-O header](https://github.com/apple-oss-distributions/xnu/blob/main/EXTERNAL_HEADERS/mach-o/loader.h)
and [fixup-chain header](https://github.com/apple-oss-distributions/dyld/blob/main/include/mach-o/fixup-chains.h).

Resolver adapters are selected by exact install name. Explicit sysroot/search
paths and @loader_path/@executable_path/@rpath are supported for graph discovery,
with cycle bounds. Intel MH_DYLIB dependencies are mapped once by canonical
path. The maintained object parser decodes public exports from tries or external
nlist entries; private/local nlist symbols are never fallback exports. Two-level
ordinals select a provider, whose explicit reexports may forward the request.
Ordinary image symbol addends are bounded against the final provider. Cyclic
dependency graphs and weak/resolver exports used by a binding are rejected.
All imports resolve before mapping; every image is relocated and every callback
validated before any constructor runs. Constructors follow dependency order,
with admitted Objective-C registration and superclass-first `+load` before C
constructors; ordinary sends trigger lazy superclass-first `+initialize`.
See [the versioned contract](abi/objc-abi.md) for admission and teardown boundaries.
Process exit registrations run in reverse registration order before image
finalizers. There is no fallback to loading macOS imports as Windows DLLs.

## Runtime dynamic loading

`dlfcn` adapters forward to a per-worker registry initialized with snapshots of
the startup graph. `dlopen` either selects an adapter-only group by exact
install name, or resolves a path (absolute, `/C/...`-style, relative, leaf name
in the working directory, sysroot `usr/lib`, or configured search directories)
and runs the same library preparation, mapping, relocation and import checks as
startup. A canonical path that is already open returns the existing handle with
an incremented reference count; `RTLD_NOLOAD` succeeds only in that case. A
requested image that is already a dependency of another active group, or a
dependency that overlaps one, is rejected instead of being shared or aliased.

Each open group records the root image, its root-first dependency order, its
adapter libraries, `RTLD_GLOBAL`/`RTLD_LOCAL` and `RTLD_FIRST` flags, and a
mapped-program pointer. Public exports are snapshotted as absolute relocated
addresses, so `dlsym` never borrows the loaded program after publication.
Executable ranges are added to the exit-callback validator before the handle is
published; constructors run after publication and use the ordinary guest
initializer path. `dlsym(handle, name)` prefixes `_` and searches the root, then
dependencies unless `RTLD_FIRST`, then that group's adapter libraries.
`RTLD_DEFAULT` searches active global groups in load order, and
`RTLD_MAIN_ONLY` searches only the main executable's public exports.
`dlerror` stores a per-thread message that is returned once; a subsequent call
returns null until another failure.

`dlclose` decrements the reference count. At zero it marks the group inactive
and, outside the registry lock, drains the group's `__cxa_atexit` registrations
per image (their DSO argument is the image header) and then runs image
finalizers in reverse order. Mapping is deliberately retained: guests may have
copied function or data pointers, and the first slice does not track them.
Groups still open at process teardown run their finalizers in reverse creation
order. Dynamic TLS, `@rpath` expansion, `RTLD_NEXT`/`RTLD_SELF`, `dladdr`,
interposition and ObjC registration remain unsupported. See
[the evidence](reverse-engineering/dynamic-loading-evidence.md) and
[the results](dynamic-loading-results.md).

## Mach IPC

The D2 adapter separates message routing from guest-visible names. Each worker
allocates local names for task-self, bootstrap and its own receive/send rights;
a receive port also has a global id made of a broker connection number and a
sequence, so the broker can address it without sharing guest name spaces.
Messages between ports in one worker bypass the broker; other destinations go
over a loopback connection authenticated by a 128-bit token in the endpoint
file. The broker forwards the payload and retires global ids when a receive
right is destroyed or a worker disconnects.

`mach_msg` validates header size, dispositions, options, notification ports and
the 16 MiB cap before sending. A successful send consumes `MOVE_SEND` or
`MOVE_SEND_ONCE` rights; `COPY_SEND`, `MAKE_SEND` and `MAKE_SEND_ONCE` behave as
their names require. On receive, the adapter reproduces XNU's copyout
inversion: the header's remote/local port-type fields are swapped, the local
name is the receiving port, and the reply name is a newly materialized send or
send-once right. `MACH_RCV_LARGE` leaves an oversized message queued for a
retry; without it the message is dropped and `MACH_RCV_TOO_LARGE` returned.
Timeouts use a monotonic deadline. A lost broker connection fails pending
operations instead of blocking forever.

The broker delivery callback only appends to a process-wide inbox and wakes a
condition variable; guest Mach state is never taken on the reader thread, which
prevents a send/receive deadlock between workers. `bootstrap_check_in` creates
the receive right and registers the service name; `bootstrap_look_up` returns a
send right. Both use the process-local service table when no broker endpoint is
configured. Complex messages, port descriptors, OOL memory, vouchers, trailers,
port sets, notifications and introspection are not implemented. See
[the ABI evidence](reverse-engineering/mach-ipc-evidence.md) and
[the results](mach-ipc-results.md).

## launchd-style jobs

The D3 broker optionally loads property lists passed with `--job`. It parses the
documented subset (`Label`, `Program`, `ProgramArguments`, `MachServices`,
`EnvironmentVariables`, `StandardOutPath`, `StandardErrorPath`, `RunAtLoad`) and
rejects every other key before publishing its endpoint. Labels and advertised
services must be unique across jobs. Names in `MachServices` are reserved in the
broker's service table; the job does not own a receive right until its guest
checks in.

A lookup for a reserved name with no live owner spawns
`adapter-run <Program> -- <arguments>` with the broker endpoint and the job's
environment. `RunAtLoad` jobs are spawned when the broker starts. The broker
tracks `Idle`, `Starting`, `Running` and `Failed`, waits up to 15 seconds for
check-in, allows one respawn after an exit during that wait, and returns
`BOOTSTRAP_UNKNOWN_SERVICE` when activation fails. A watcher thread marks the job
`Idle` when the child exits. The broker does not terminate running children on
shutdown; they observe the lost namespace on their next Mach call.

This is an Adapter control plane, not launchd. There is no PID 1, `launchctl`,
`launch_msg`, KeepAlive, throttling, signal supervision, socket/event
activation, privileged or per-user domain, or launchd-precreated receive right.
See [the launchd evidence](reverse-engineering/launchd-bootstrap-evidence.md) and
[the results](launchd-results.md).

## XPC

The D4 adapter resolves `/usr/lib/system/libxpc.dylib` and implements libxpc's
public object model over an adapter-defined transport. Type constants and the
two connection errors are exported static marker objects, matching the header
rule that they are addresses rather than pointer variables. Live objects live
in a registry with atomic reference counts; containers retain children, and
strings, data, arrays and dictionaries own copies of their contents. Messages
are serialized at send time with a little-endian tagged format, a depth cap of
64, a 16 MiB total/value cap and a 1 MiB string/key cap.

A listener connects by checking a service name into the same bootstrap table
used by D2/D3. A client connection asks the broker for that name; the broker
creates two route ids and a link, then delivers an accept event carrying the
server's route. Messages and replies travel as broker frames validated against
that link, so a route cannot address an unrelated worker. A single process-wide
dispatcher thread invokes each connection's event-handler block, preserving
per-connection order; target queues are recorded but not executed, and there is
no GCD scheduling. Cancel releases the service port, closes the route and
delivers `XPC_ERROR_CONNECTION_INVALID` locally and
`XPC_ERROR_CONNECTION_INTERRUPTED` to the peer.

The public boolean-returning adapters return `i32` 0/1: Rust `bool` returns do
not define the upper `%eax` bytes, and C callers may read the whole `int`.
Dates, UUIDs, file descriptors, shared memory, endpoints, rich errors,
`xpc_main`, peer requirements and sandbox/entitlement enforcement are absent.
See [the XPC evidence](reverse-engineering/xpc-evidence.md) and
[the results](xpc-results.md).

## Darwin thread support

On-disk TLV descriptors use the 24-byte function/key/offset layout described by
Apple's [ThreadLocalVariables.h](https://github.com/apple-oss-distributions/dyld/blob/main/libdyld/ThreadLocalVariables.h).
Validated template bytes are copied after relocation and instantiated lazily
per image/thread with explicit alignment. The thunk returns in RAX and restores
other registers, using XSAVE/XRSTOR for enabled extended state (up to 16 KiB),
or FXSAVE/FXRSTOR on hosts without OS-enabled XSAVE. Stack pages are probed and
state buffers aligned. This preserves a stricter ABI than ordinary sysv64;
linked assembly probes retain GPR, SSE and AVX values across TLV access. Shared
cache/preinitialized thunks remain unsupported. Image-provided TLV initialization
code executes as ordinary guest code, and `__tlv_atexit` registrations are
drained per thread in LIFO order.

The worker links with an 8 MiB main-thread stack reserve (`/STACK:8388608`) to
match Darwin's default. Windows MSVC otherwise reserves 1 MiB; jq's upstream
recursion tests exhausted that smaller stack. This is a measured environment
match, not proof that every guest stack workload is equivalent.

Owned Windows-hosted threads invoke guest start functions using SysV. Join
returns their pointer-width results. A running-thread count prevents mappings
being released even if another guest thread has consumed the corresponding
join handle. Unjoined owned threads are waited for before teardown; this differs
from Darwin process exit and a nonterminating thread reaches the worker timeout.
Stack-size pthread attributes are honored; other attributes, Apple pthread_t
internals and pthread cleanup-stack macros are not exposed. Cancellation is
unsupported.

Thread-specific keys have per-thread values and four destructor passes. Once
controls use their address and the observed Darwin initializer signature; the
opaque storage is not replaced with a Windows structure. Initialization occurs
outside locks while other callers wait. A separate errno integer belongs to
each thread. Condition variables support static initialization, wait,
absolute-deadline timedwait, signal and broadcast over host condition variables;
process-shared use, cancellation and fork interactions remain unsupported.
C++ TLV constructors and destructors are implemented, including cleanup-time
re-registration; ordering against key destructors and image finalizers is an
adapter choice rather than a verified Darwin ordering.

## Darwin libc data and I/O

`build.rs` compiles C adapters and small assembly entry thunks with Clang and
archives them with llvm-ar. Public guest calls use explicit SysV attributes;
host helper calls use Windows conventions. FILE (152 bytes), RuneLocale (3,208
bytes), stat64 (144 bytes), tm (56 bytes) and SysV va_list (24 bytes) have compile-
time offset/size checks. A separate CRT FILE pointer follows each guest FILE.
The three standard-stream symbols export addresses of pointer variables.

Guest I/O is binary, including inherited stdin. The `/C/...` drive namespace
retains POSIX absolute-path recognition for unmodified consumers. Windows
metadata is translated into the Darwin stat structure with documented synthetic
fields. A UTF-8 guest environment preserves case-sensitive names and empty
values. UTC conversion uses signed Gregorian arithmetic; local time and common
math/format primitives use typed host operations. Unsupported locale names and
formatting variants fail instead of claiming those semantics.

See [the ABI evidence](reverse-engineering/jq-libc-abi.md),
[API contracts](compatibility.md) and [the measured milestone](jq-libc-results.md).

## Evidence and platform choices

REA CLI 6.4.0 is installed. The current client has no REA MCP registration.
Ghidra 12.1.4 was installed from its SHA-256-verified GitHub release and is
reported healthy by `rea doctor` (Java 21), but the Windows provider rejects
Mach-O targets with `target_format_unsupported` ("Windows Ghidra P0 accepts PE
targets only"). Hopper is unavailable on win32 and no IDA registration exists.
Consequently Adapter's Mach-O evidence remains its own bounded inspector plus
LLVM header/disassembly output; no REA decompilation of the target is claimed.
Earlier REA attempts also returned a root-only DMG inventory and unsupported
XIP. Portable REA directory inventories succeeded on selected extracted trees.
See the retained evidence in `docs/reverse-engineering/`.

The supplied Xcode 27 launcher, compiler and sampled command tools are ARM64
only, while selected Additional Tools contain x86-64 slices. Keep ARM translation
as a separate future architecture: changing calling conventions does not
translate ARM instructions. These packages are targets/evidence, not a source
of Windows-ready macOS system framework implementations. No Apple binaries are
vendored or committed, and archive checksums do not establish signer trust.
