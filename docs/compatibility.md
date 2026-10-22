# Measured support

Implemented means code exists. Verified means listed tests have actually passed.
Compatible is reserved for an identified artifact and measured behavior; no
general macOS application compatibility is established.

| Feature / target | Implemented | Verified evidence | Compatibility boundary |
|---|---|---|---|
| Thin little-endian x86-64/arm64 Mach-O inspection | Yes | Generated files, 43 Apple images, published jq | Static inspection only |
| FAT32 x86/arm selection | Yes | Universal fixture and Apple universal apps | x86 slice selected; other slice internals not inspected |
| FAT64 selection | Yes | Synthetic selection unit test | No universal FAT64 application execution fixture yet |
| Malformed/truncated inputs | Bounded checks | Rust tests, mutation probe, integration negatives | No security-hardening certification |
| Dependency graph / install names / rpaths | Yes | Local/transitive graphs, missing library, adapter graph | Cycles rejected |
| Intel dylib execution / exports / reexports | Yes | Classic/chained three-image consumers, facade and duplicate-name probes, runtime plugins | No weak coalescing, resolver exports, interposition or cross-group dependency sharing |
| Windows mapping / SysV entry / imports | Yes | return37, hello, ABI and libc fixtures, original jq | Supported Intel PIE/load-command subset |
| Classic dyld rebases and pointer binds | Yes | hello, CF probes | No weak/interposed/flat namespaces |
| Chained fixups / offset initializers | Partial | hello-chained, CF-chained | Only emitted subset tested |
| Initializers and image cleanup | Yes | Ordered cross-image constructors, callbacks/destructors and repeated workers | No Darwin process teardown equivalence |
| libSystem adapters | jq/ripgrep-oriented subset | Original jq and ripgrep workflows plus libc/ABI/lifecycle/thread probes | API subsets documented below; full libc semantics unverified |
| Darwin TLV/TLS | Partial | Nonzero/zero/aligned/pointer templates; eight threads across two images; register probes; C++ constructors and thread-exit destructors through `__tlv_atexit` | No shared-cache TLVs, `__cxa_thread_atexit` path or exception unwinding |
| pthread lifecycle / thread state | Partial | Create/join/self/equal, once, keys/destructors, errno isolation, stack-size attributes, naming and thread IDs, normal/recursive mutexes, condition variables and condattrs | No cancellation, errorcheck/adaptive mutexes or process-shared condition variables |
| CF immutable UTF-8 strings | Five functions | Guest UTF-8/UTF-16 and ownership fixture | Runtime-created opaque strings only |
| Bundle executable discovery | Yes | Hello.app, traversal rejection | Resource APIs/GUI absent |
| Runtime dyld loading | Partial | A Mach-O consumer loads classic, chained and MH_BUNDLE plugins after startup, resolves function/data/string exports through handles, observes constructors/destructors, duplicate and `RTLD_NOLOAD` handles, `RTLD_LOCAL`/`FIRST` scope and per-thread one-shot errors | Images are never unmapped; dynamic TLS and `@rpath` arguments rejected before constructors; no `RTLD_NEXT`/`RTLD_SELF`, `dladdr`, dependency sharing across groups, interposition or ObjC registration |
| Mach IPC / bootstrap | Partial | Two unchanged Mach-O processes over a loopback broker; send/receive, send-once replies, retirement and failure semantics in local and broker modes | Simple messages only; no complex messages, port sets, notifications, introspection or launchd-precreated receive rights |
| launchd jobs | Partial | Broker loads a documented plist subset, reserves `MachServices`, starts jobs on lookup or `RunAtLoad`, applies environment/log paths, waits 15 seconds for check-in, respawns after exit and reports spawn failure as unknown service | No launchd process, `launchctl`/`launch_msg`, KeepAlive/throttling/supervision, socket/event activation, privileged or per-user domains; check-in creates the receive right |
| XPC | Partial | An unchanged libxpc client/server pair over the broker exchanges a typed dictionary request/reply (bool/int64/double/string/data/array/dictionary), uses listener check-in, a sync reply, remote-connection reply creation, cancel, peer interruption and missing-service invalid events; a broker job starts the listener on demand | Adapter-defined serialization and transport, not Apple's wire format; no real-macOS peer interoperability, `xpc_main`/GCD semantics, date/UUID/fd/shmem/endpoint/rich-error objects, peer validation or sandbox/entitlement enforcement |
| Objective-C | Bounded raw-isa MRC startup runtime | Real class/method consumer, classic/chained inheritance/ownership/dispatch tests and pre-constructor metadata rejection | No categories/protocols, relative methods, ARC/weak, dynamic registration, exceptions or aggregate/x87 sends; no Foundation/AppKit |
| jq macOS amd64 1.8.2 | Demonstrated tested workflows | 13 CLI methods plus the pinned upstream `jq.test` suite: 550/550 with module fixtures, debug and release | No reference-Mac comparison; suite excludes upstream shell/`man.test` harnesses |
| ripgrep macOS amd64 15.2.0 | Demonstrated tested workflows | 11 CLI methods: version/help, literal/regex/context, ignore/hidden, JSON, PCRE2/UTF-8, stdin, binary, 2 MiB mmap, 40-file threads, exit codes, clean `--pre` failure; debug and release | No reference-Mac comparison; this target does not exercise condition variables; process spawning and signal delivery are absent |
| Xcode 27.0 build 27A266 | Inspection only | XAR Content checksum, ARM64 header, startup disassembly | Not executable / not compatible |
| Sampled CLT 27 executables | Inspection only | 13 ARM64-only images | Not executable / not compatible |
| Additional Tools / Icon Composer | Inspection only | Intel/ARM slices and actual dependencies | Not executable / not compatible |

The C/Objective-C probe executables are real Mach-O files cross-compiled for
macOS by Clang and linked by ld64.lld on Windows, then executed unchanged by
adapter-run. They are neither PE recompilations nor mocked interpreters. They have
not all been run on a reference Mac. A separate 4,096-case libc consumer has
same-artifact Mac/Windows evidence in [differential results](differential-testing.md).
Successful probe execution demonstrates only
their tested machine-code/API behavior.

## Implemented API contract

All functions below use Darwin x86-64 SysV calling convention. Darwin `long` /
CFIndex and size_t are 64-bit even though Windows C long is 32-bit.

| API | Windows-side behavior | Error/semantic boundary | Tests |
|---|---|---|---|
| puts | Writes raw bytes and LF through the stdout adapter | Nonnegative on success, -1 on I/O failure | hello, libc and jq output |
| strlen | Counts bytes before NUL | Caller must provide a valid C string | hello and ABI |
| malloc / free | Host CRT allocation/free through ABI adapters | Standard valid-pointer contract; guest alignment probed indirectly only | hello allocation/use/free |
| memcpy | Copies non-overlapping bytes | Caller supplies valid buffers; not memmove | hello and ABI |
| memcmp | Returns signed first-byte difference | Unsigned byte comparison; valid buffers required | hello and CF |
| CFStringCreateWithCString | Copies valid UTF-8 into owned Rust string | Only null/default allocator and UTF-8 supported; returns null for other encodings/invalid UTF-8 | CF consumer |
| CFStringGetLength | Counts UTF-16 code units | Runtime-owned handles only; no NSString bridging | BMP + surrogate-pair check |
| CFStringGetCString | Copies UTF-8 plus NUL if buffer fits | Returns false for small buffer/unsupported encoding | Exact fit and one-byte-short buffer |
| CFRetain / CFRelease | Count ownership of runtime-created strings | No Apple constant strings, custom allocator, other CF types, or toll-free bridging | Retain/release/use/release |
| dlopen / dlclose | Maps an Intel dylib or bundle after startup, validates every import, runs constructors in dependency order and finalizers at close; duplicate canonical paths return the same refcounted handle; `RTLD_LOCAL`, `GLOBAL`, `NOLOAD` and `FIRST` are honored; `RTLD_NODELETE` is accepted because images always remain mapped | `@rpath`/`@loader_path`/`@executable_path` arguments are rejected; dynamic TLS, dependency overlap with another active group and handle aliasing of a startup dependency are rejected before constructors; close never unmaps code | Host consumer over classic, chained and bundle plugins; TLS rejection and non-executable input negatives |
| dlsym / dlerror | Prepends `_` to C names; a handle searches its root, then dependencies unless `RTLD_FIRST`, then its adapter libraries; `RTLD_DEFAULT` searches active global groups in load order; `RTLD_MAIN_ONLY` searches main-image public exports; errors are per-thread and returned once | No `RTLD_NEXT`/`RTLD_SELF`, `dladdr`, symbol interposition or search of inactive groups; adapter groups expose only registered adapter symbols | Plugin lookup of function, data and string exports; scope, stale-handle, missing-file and one-shot error checks |
| __cxa_atexit / __cxa_finalize / atexit / exit | Executable callbacks, DSO filter, reverse registration order; exit flushes host streams | Explicit exit terminates worker after callbacks; full signals/termination semantics absent | Cross-image order, finalize-twice and explicit exit/flush consumers |
| pthread_create / pthread_join | Own host threads and bridge guest start/return pointers | Darwin attributes accepted for stack size (16 KiB to 512 MiB); 128 running threads / 4096 total starts; Darwin error numbers | Eight threads, error returns and return values; ripgrep's worker pool |
| pthread_attr_init/destroy/setstacksize | Validated opaque Darwin-attribute prefix and host stack reservation | Only stack size is honored; guard-size and scheduling attributes are not modelled | Attribute unit tests; ripgrep thread creation |
| pthread_get_stackaddr_np / pthread_get_stacksize_np | Current host thread limits | Windows-provided bounds, not Darwin's reserved secondary-stack layout | Attribute/stack fixture and ripgrep |
| pthread_setname_np / pthread_threadid_np | Win32 thread description and thread ID | Names truncated at 63 UTF-8 bytes; IDs are Windows TIDs | Thread fixture |
| pthread_mutex_init/lock/trylock/unlock/destroy | SRWLOCK-backed normal and recursive mutexes with owner/depth tracking | No errorcheck/adaptive types, robust/process-shared mutexes or Darwin private layout | Recursive-depth, owner-error, type-rejection and ripgrep synchronization tests |
| pthread_cond_init/destroy/wait/timedwait/signal/broadcast | Windows condition variable over the SRWLOCK inside the guest mutex; static `PTHREAD_COND_INITIALIZER` works without an init call | Normal (non-recursive) mutexes only; absolute realtime deadlines; destroying with waiters is undefined; no cancellation | Signal, broadcast, four waiting workers, timedwait timeout/past deadline/invalid timespec and static-initializer consumers |
| pthread_condattr_init/destroy/getpshared/setpshared | 16-byte Darwin condattr with default `PTHREAD_PROCESS_PRIVATE` (2); accepts shared (1) and private (2) values | Process-shared value is recorded but host condition variables cannot be used across processes | Default, set/get round-trip and invalid-value rejection |
| __tlv_atexit / ___dso_handle | Registers per-thread C++ destructors, runs them LIFO at thread exit and drains registrations added during cleanup | DSO argument is accepted and ignored; no `__cxa_thread_atexit` path; main-thread destructors run at session teardown | Apple Clang `thread_local` fixture: three worker threads plus the main thread |
| sigaction/signal/sigprocmask/sigaltstack/raise/pause | Records Darwin handlers, masks, flags and alternate stack; rejects SIGKILL/SIGSTOP | Handlers are never delivered and pause blocks forever; no Darwin signal/SEH integration | Rust std startup path only; no delivery test |
| fork/execvp/waitpid/setpgid/setsid/setgroups/posix_spawn* | Explicit ENOSYS/ECHILD failures; spawn action/attribute objects are opaque and validated | No child process creation or process-group/credential changes | ripgrep `--pre` fails cleanly with exit 2 |
| pthread_self / pthread_equal | Stable per-session identity tokens | Apple pthread structure internals unavailable | Worker/main identity checks |
| pthread_key_create/delete/getspecific/setspecific | Per-thread key values and up to four destructor passes | 512 live keys; keys not reused; no reserved/private keys | Reentrant destructor rearming, thread isolation and deleted-key errors |
| pthread_once | Per-control initialization and waiting callers | Static controls with Darwin initializer signature; no cancellation/fork support | Eight concurrent callers, repeated calls, visibility of initialized data |
| __error | Stable pointer to a per-thread 32-bit errno | Common CRT/Windows errors translated; full mapping absent | Thread isolation, missing files and API argument errors |
| _NSGetExecutablePath | Returns the guest path supplied in argv[0]; on a short buffer, reports the required byte count without writing | Windows path spelling, not a `/C/...` guest path; no symlink canonicalization | libc fixture size-query and exact argv[0] match |
| mach_task_self_ / bootstrap_port | Exported data symbols holding stable per-worker names | Values are adapter names, not kernel task/bootstrap ports; no privilege checks | Mach failure fixture |
| mach_port_allocate / deallocate / mod_refs / insert_right / destroy | Receive-right allocation, send/send-once reference counting, MAKE/COPY dispositions, dead-right tracking and broker retirement | Receive rights only from allocate; no port sets, dead-name requests, guarded or strict rights, or task ports other than self | Local and broker-backed Mach failure fixture; cross-process retirement |
| mach_msg | Sends simple messages and receives with XNU header inversion, `MACH_RCV_LARGE`, `MACH_RCV_TIMEOUT` and 16 MiB cap; reply rights materialized in the receiver | Complex bits rejected; no OOL, descriptors, vouchers, trailers, port sets or notifications; no strict reply | Echo, header-bit, timeout, LARGE-retry, refcount and stale-right assertions |
| bootstrap_check_in / look_up | Broker-owned service namespace; check-in allocates a receive right, duplicate reports 1103, lookup returns a send right or 1102; a lookup for a registered job's service activates the job and waits for check-in | Process-local fallback without `ADAPTER_MACH_BROKER`; no launchd-precreated port, register/subset or privileged namespace | Duplicate check-in, absent lookup, bad bootstrap port, cross-process echo, on-demand job activation |
| adapter-broker | Loopback TCP endpoint file with a random token; routes payloads between local workers and owns service names and retirement; optionally loads job plists and spawns registered jobs | Trusted-local-worker coordination and job control, not a security boundary; dropped connections fail pending operations instead of hanging; running jobs are not killed on broker exit | Broker-backed failure fixture, cross-process test and broker job tests |
| adapter-broker `--job` | Loads `Label`, `Program`, `ProgramArguments`, `MachServices`, `EnvironmentVariables`, `StandardOutPath`, `StandardErrorPath` and `RunAtLoad`; spawns `adapter-run` on lookup or at load, waits 15 seconds for check-in, allows one respawn and reports spawn failure as 1102 | Unknown keys fail startup; no `launchctl`/`launch_msg`, KeepAlive, supervision, throttling, sockets, events, per-user domains or `StandardInPath`/umask/cwd | On-demand spawn and respawn, RunAtLoad, spawn failure and unsupported-key rejection |
| xpc objects (null/bool/int64/uint64/double/string/data/array/dictionary) | Registry-backed objects with reference counting; containers retain children; type markers and static bool/error singletons match the header address model; typed setters and getters, append/set-by-index, apply for arrays and dictionaries, `xpc_equal`/`hash`/`copy`/`copy_description` | No date, UUID, fd, shared memory, endpoint or rich-error objects; booleans cross the ABI as `i32` 0/1; values are limited to 16 MiB and depth 64 | Typed request/reply validation in the client fixture |
| xpc connections | `create_mach_service` for listeners and clients, event handler blocks on a process-wide dispatcher, activate/resume/suspend/cancel, async send, reply-handler and synchronous sends, reply dictionary creation, remote-connection retrieval and `get_name`; listener check-in uses the bootstrap table and broker links | No anonymous connections, `xpc_main`, GCD target-queue execution, peer code-signing/entitlement checks, audit tokens or macOS wire interoperability; cancellation is local plus a broker notice | Echo, reply typing, cancel/peer-interruption, missing-service and on-demand-launch consumers |

## jq-oriented libc slice

The original [jq 1.8.2 release artifact](https://github.com/jqlang/jq/releases/tag/jq-1.8.2)
is pinned by SHA-256 in `tools/fetch_jq.py`. It is downloaded into ignored test
storage, not rebuilt or included in the repository. Its 1,357 fixups now pass
preflight. All imports bind; this is not a claim that every path is equivalent
to Darwin. Exact evidence and data offsets are in
[the ABI report](reverse-engineering/jq-libc-abi.md).

| Surface | Implemented contract | Limits / evidence |
| --- | --- | --- |
| RuneLocale / maskrune / setlocale | 3,208-byte LP64 layout; cached ASCII classification and case maps, C/POSIX locale and query/environment selection | Unsupported names fail without changing locale. No Unicode/xlocale implementation |
| FILE / standard streams | 152-byte Darwin prefix backed by a separate CRT FILE; exported pointer variables; zero inline counts select slow paths | No general cookie callbacks, user-supplied FILE objects or Darwin private locking state |
| fread/fwrite/getc/fgets/fputc/fputs/puts | Binary streams preserve CRLF, NUL and high bytes; element counts, EOF/error flags, clearerr, fileno, flush and close | Valid pointer/stream contracts; broad buffering/locking features absent |
| printf/fprintf/snprintf/vsnprintf/sscanf/vsscanf | SysV GP/SSE/stack extraction; LP64 integers; width/precision; common integer, char/string, scanset, pointer, floating and count conversions; truncation/length behavior | Positional, wide and long-double formats return ENOTSUP. Field width/precision capped at 1 MiB. Some host formatting details differ |
| UTF-8 files | fopen/fdopen aliases, selected Darwin open flags, close; `/C/...` drive mapping and canonical paths | Directory descriptors, symlink/permission fidelity, descriptor inheritance and complete flag set absent |
| stat$INODE64 / fstat$INODE64 | Explicit 144-byte structure translation; file kind, size, IDs and timestamps | uid/gid zero; permissions synthesized; ctime uses modification time; no full Darwin metadata fidelity |
| Environment | UTF-8, case-sensitive guest environ/getenv/setenv/unsetenv; empty values retained; TZ forwarded to time backend | Direct environ mutation and concurrent unsynchronized pointer use unverified |
| Time | LP64 time/tm/timeval; Gregorian UTC conversion incl. negative epoch, normalization; selected strptime/strftime formats | Local time relies on Windows CRT ranges/TZ rules. Date formats and zone strings incomplete |
| Allocation/string/math | jq's remaining imported adapters, heap sort with SysV comparator, CSPRNG and stack probes | No complete numerical/errno/fenv equivalence. `nexttoward` bridges the incoming x87 80-bit argument rather than treating it as a Windows double |

Focused classic/chained ABI probes test register/stack varargs, more than eight
floating arguments, 64-bit long, explicit va_list, truncation, count stores,
unsupported formats, extended-precision nexttoward direction, stream data and
exit callbacks. A second consumer tests UTF-8 paths, binary file writes/reads,
guarded stat buffers, errno and UTC conversions. Real jq tests exercise regex,
Unicode, streaming, files, modules, sorting/grouping, dates, math and errors.

### ripgrep 15.2.0

The published [ripgrep 15.2.0 macOS amd64 release](https://github.com/BurntSushi/ripgrep/releases/tag/15.2.0)
is pinned by SHA-256 in `tools/fetch_ripgrep.py`. Its 131 API imports are all
bound (the 132nd symbol is `dyld_stub_binder`, handled by the eager binder
diagnostic). The second target forced adapters for directory `fdopendir`/`dirfd`,
`MAP_FIXED` and file-backed `mmap`, `posix_memalign` overrun safety, `F_DUPFD_CLOEXEC`,
`_SC_HOST_NAME_MAX`, `memset_pattern16`, `___chkstk_darwin`, dispatch semaphores,
Darwin signal recording, pthread attributes/stack limits/naming/IDs and
normal/recursive mutexes. Eleven tests cover version/help, literal and regex
search, context, gitignore and hidden rules, JSON, PCRE2 and UTF-8, stdin,
binary detection, a 2 MiB memory-mapped file, 40-file multithreaded listing,
exit codes, and a clean `--pre` failure. All pass in debug and release; see
[results](ripgrep-results.md) and the [target evidence](reverse-engineering/ripgrep-target.md).

Process creation is deliberately absent: `fork`, `execvp`, `waitpid` and
`posix_spawnp` return ENOSYS/ECHILD instead of starting a child. Signal handlers
are stored but not delivered. Consequently ripgrep's preprocessor feature fails
with exit 2 rather than executing a command.

No Foundation classes are implemented. The dyld_stub_binder symbol resolves to
an explicit terminating diagnostic; it is not a lazy linker. All supported lazy
slots are bound eagerly before guest execution.

Mach IPC and XPC use a separate `adapter-broker` process; `ADAPTER_MACH_BROKER`
must name its endpoint file for cross-process ports, bootstrap services and XPC
routes. Without it, a worker has a process-local namespace and only self-send
succeeds, and XPC connections fail as invalid. The broker routes simple Mach
messages and typed XPC payloads, owns service names and starts registered jobs.
It does not enforce rights, provide memory protection, validate entitlements or
authenticate peer code. See [mach-ipc-results](mach-ipc-results.md),
[xpc-results](xpc-results.md), the
[Mach ABI evidence](reverse-engineering/mach-ipc-evidence.md) and the
[XPC evidence](reverse-engineering/xpc-evidence.md).

Latest commands/results are recorded in [test-results/latest.json](test-results/latest.json).
The current run passed all 16 verification stages, including 112 integration
tests in both profiles. The latest iteration and remaining limits are described
in [jq-libc-results](jq-libc-results.md),
[ripgrep-results](ripgrep-results.md),
[dynamic-loading-results](dynamic-loading-results.md),
[mach-ipc-results](mach-ipc-results.md) and
[xpc-results](xpc-results.md).
