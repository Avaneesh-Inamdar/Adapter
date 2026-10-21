# Limitations and risk register

| Risk | Current status / mitigation | Next evidence needed |
|---|---|---|
| ARM64 Xcode/toolchain on x64 host | All sampled core Xcode/CLT images ARM64 only; execution rejected | A separately scoped instruction translator or Intel artifact |
| Apple Objective-C ABI | Metadata detected; no runtime implementation | Class/selector layouts, initialization/dispatch/ownership tests with real consumers |
| Framework surface and GUI | Foundation/AppKit/SwiftUI absent | Minimal real app requirements and reference execution |
| Darwin system calls / Mach services | No syscall interception or kernel personality | Controlled syscall probe and Windows adapter model |
| TLS and pthread semantics | Basic TLV templates, threads, keys, once, errno, stack-size attributes, naming and normal/recursive mutexes verified | C++ TLV lifecycle, cancellation, condition variables and reference behavior |
| Dynamic libraries | Local Intel graph execution and reexports verified; cycles rejected | Cyclic initialization, weak/interposed exports, resolver functions, runtime dlopen/dlclose |
| Exceptions/unwind/signals | No Darwin unwinder or signal model | Cross-image C++/ObjC throws, Windows SEH interaction |
| Modern dyld variants | Chained formats restricted; no weak/flat binding | Real target-specific encodings + negative regression corpus |
| ABI differences | SysV calls plus measured printf-family varargs and x87 argument adapters | General aggregates/vectors, wide/positional/long-double formatting, exception/unwind interaction |
| Runtime semantic equivalence | Probes and original jq/ripgrep workflows; no reference Mac | Same-artifact reference output and API error comparisons |
| Untrusted code / parsing | Bounds/resource caps; worker timeout; not a security sandbox | Dedicated fuzzing and hardened execution isolation |
| Apple authentication | Embedded content checksums verified where reported; signer trust unverified | Independent trusted Apple signature/certificate validation |
| REA limitations | Portable inventories work; Ghidra is installed but the win32 provider accepts PE only; Hopper needs macOS and IDA is unconfigured | A supported engine/host; no invented decompiler findings |
| Guest stack reserve | Worker now reserves 8 MiB like Darwin's main thread; MSVC default 1 MiB overflowed jq recursion | Deep-recursion consumers beyond jq's suite; secondary-thread stack equivalence |
| Local execution policy | An earlier milestone's launches were denied by Windows Application Control (4551); the current full run passed without policy changes | Record any future policy blocks; no bypass performed |

Unsupported executable formats include Mach-O32, swapped-endian thin files,
ARM execution, non-PIE files, LC_UNIXTHREAD entry, encrypted/protected images,
section relocations, shared-cache/preinitialized TLVs, TLV initialization
functions and Objective-C metadata. Code signatures
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

Condition variables, networking, resource lookup APIs, fork/exec, security
services, Metal, AppKit event loops, and runtime dlopen are absent. Signal
handlers and alternate stacks are recorded but never invoked; pause blocks.
pthread stack-size attributes are honored, but guard size, scheduling and
cancellation are not. ripgrep's preprocessor path therefore fails cleanly with
exit 2 instead of starting a child process.
Xcode's dynamically loaded application libraries make its five direct launcher
dependencies a lower bound, not its complete dependency closure.

Darwin errno storage and common error translation exist; the mapping is incomplete.
Unjoined guest threads are waited for before unmapping rather than terminated
as Darwin exit would do. Nonterminating workers are stopped by the parent
timeout. pthread_t tokens do not implement Apple's internal structure, cleanup
macros or cancellation. Once controls support the public static initializer
contract, not inspection/modification of private opaque state. C++ TLS
initializers/destructors and exceptions remain unsupported. XSAVE state over
16 KiB causes a controlled rejection for programs using TLV templates.

The archive tools create fresh output directories, copy only chosen regular
files, record symlink targets without creating links, bound chunk sizes, and
verify stored payload checksums. They support the observed XAR/PBZX/odc
encodings, not arbitrary Apple installer formats. 7-Zip reported HFS image tail
data warnings. Its XAR PackageInfo/Distribution extraction produced payload
aliases; those copies were discarded as evidence, and the package was decoded
directly from its XAR TOC with checksum validation instead.
