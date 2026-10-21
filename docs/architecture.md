# Architecture and decisions

## Measured environment

Audit date: 2026-10-21 (Asia/Calcutta). Workspace `E:\Xcode` was empty and was
not a Git repository; initialized Git without overwriting existing source.
Host: Windows build 10.0.26300, x64. Rust 1.99.0 MSVC target, Python 3.12,
LLVM 23.1.3, MinGW GCC 16.2.0, and CMake 4.4.3 were observed. The implemented
build uses Cargo/MSVC plus Clang C/assembly ABI adapters, not MinGW/CMake. Clang and ld64.lld are installed outside
PATH. No reference Mac is available.

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
| `src/darwin` | Rust pthread lifecycle/state, recursive mutexes, condition variables, C++ TLV destruction, dyld image introspection, TLV ABI bridge, Darwin stack probe and exit callbacks |
| `src/darwin/host` | C adapters for libSystem stdio/files/stat/time/locale, signals, process stubs, dispatch, mmap and directory iteration |
| `src/frameworks/corefoundation.rs` | Five opaque immutable-string/ownership adapters |

Execution stages: discover -> inspect graph -> reject unsupported requirements
-> decode/resolve fixups -> reserve/commit/copy -> apply fixups -> validate
initializer pointers -> install final protections -> initialize -> invoke
LC_MAIN -> join owned threads -> finalize -> unmap. Errors include their failing stage/symbol.
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
and process exit registrations run in reverse registration order before image
finalizers. There is no fallback to loading macOS imports as Windows DLLs.

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
