# Thread lifecycle ABI evidence

Investigated 2026-10-21. Scope: pinned Xcode 27 SDK headers plus an Apple Clang
cross-compilation probe for C++ thread-local destruction. No macOS reference
execution was available. The REA MCP native provider on this Windows host accepts
PE targets only, so no Mach-O decompiler result is claimed here; the headers and
compiler output below were produced by direct local tools. Finding IDs are local
report identifiers, not REA Evidence IDs.

## Artifact and method

- Input: `files/Xcode_27.xip`, 2,014,229,334 bytes, SHA-256
  `6a270c53a5a0c5e0ac78125342d44c3cfff2716ec390373cd5e30834d80a67c3`.
- Inventory: `target/artifacts/xcode27-inspection` (verified PBZX/XAR inventory,
  166,799 entries). `tools/read_xip_member.py` reconstructs a member's CPIO
  position from that inventory, validates the actual `070707` header and name,
  decompresses only the intersecting PBZX blocks, and writes a provenance JSON
  bound to the source hash.
- Canonical extracts: `target/artifacts/xcode27-sdk/`.

**Hardlink canary finding.** The SDK `usr/include/pthread/pthread.h` appears nine
times as a CPIO hardlink (`nlink=9`, `ino=073160` in the archive header). Only
the first member in the link group carries file data; the other eight are
`NULLcanary` placeholders of the same nominal size. The data carrier is the
XRSimulator copy:

| Field | Value |
| --- | --- |
| Member | `Xcode.app/Contents/Developer/Platforms/XRSimulator.platform/Developer/SDKs/XRSimulator.sdk/usr/include/pthread/pthread.h` |
| Reconstructed CPIO offset | `8811985391` |
| Size / SHA-256 | 28,093 / `9d621c730d1d96b600893b0e3e4c45822a24d565e7b6b166973c41a6a2eb02e7` |
| Filesystem placeholder | false |

The MacOSX SDK copy at offset `8812093396` hashes to
`6ad4c81b59f75bae894ddc626a96b62acb364229166e577843c19874d4b0750a` and begins
with `NULLcanary`. It is retained under
`target/artifacts/xcode27-sdk/placeholders/pthread.h.canary` with its provenance.
A reader that extracts only the conventional MacOSX SDK path would silently
consume placeholder bytes instead of a header.

Other members used here:

| Member (under `XRSimulator.sdk/usr/include`) | Offset | Size | SHA-256 |
| --- | ---: | ---: | --- |
| `sys/_pthread/_pthread_types.h` | `5167316425` | 3,855 | `741433612525badf767d48b1924d4c7ba564ccc5c1bab5cb7045c238850fa86b` |
| `sys/_pthread/_pthread_cond_t.h` | `5167185241` | 1,521 | `1f44e579de86408da3f83e47918e11d62d41fdce1c0230a7e30f5c56414ef914` |
| `sys/_pthread/_pthread_condattr_t.h` | `5167201044` | 1,545 | `167ad51c9e184c2f22e67dd7100e321305b11b3fe2cbeffd074015f20ce4e315` |

`pthread/pthread_impl.h` was extracted separately from the MacOSX SDK member
(offset `8812516762`, 1,977 bytes, SHA-256
`42896cbc575d9099c53f8c0afbaa04f40b2d11d24f92af5ccd7dacdaf8690231`). The
retained copy contains the signature definitions and is not a `NULLcanary`
payload; its provenance record predates the placeholder flag, so that check was
made by reading the extracted bytes.

## TL-1: condition variable and attribute layouts

**Observed, high confidence** from `_pthread_types.h` lines 35-55 and 68-76:

| Type | Fields | Opaque bytes | Total LP64 size |
| --- | --- | ---: | ---: |
| `pthread_cond_t` | `long __sig` + `char __opaque[40]` | 40 | 48 |
| `pthread_condattr_t` | `long __sig` + `char __opaque[8]` | 8 | 16 |
| `pthread_mutex_t` | `long __sig` + `char __opaque[56]` | 56 | 64 |
| `pthread_mutexattr_t` | `long __sig` + `char __opaque[8]` | 8 | 16 |

Selected headers are included through `sys/_pthread/`, not the compiler-visible
`pthread.h` alone. The 32-bit sizes in the same header (`40`/`24`/`4`) do not
apply to the Intel target.

`pthread_impl.h` defines the public static-initializer signatures:

| Constant | Value |
| --- | ---: |
| `_PTHREAD_MUTEX_SIG_init` | `0x32AAABA7` |
| `_PTHREAD_RECURSIVE_MUTEX_SIG_init` | `0x32AAABA2` |
| `_PTHREAD_COND_SIG_init` | `0x3CB0B1BB` |
| `_PTHREAD_ONCE_SIG_init` | `0x30B1BCBA` |

`pthread.h` line 214 expands `PTHREAD_COND_INITIALIZER` to
`{_PTHREAD_COND_SIG_init, {0}}`; lines 153-154 define
`PTHREAD_PROCESS_SHARED 1` and `PTHREAD_PROCESS_PRIVATE 2`. The header declares
`pthread_cond_init`, `pthread_cond_wait`, `pthread_cond_timedwait`,
`pthread_cond_signal`, `pthread_cond_broadcast`, `pthread_cond_destroy`,
`pthread_condattr_init/destroy`, `pthread_condattr_getpshared` and
`pthread_condattr_setpshared`. It also declares the non-portable
`pthread_cond_signal_thread_np` and `pthread_cond_timedwait_relative_np`, which
this adapter does not implement. **No `pthread_condattr_setclock` declaration
exists in this header or the extracted SDK members.** The timedwait deadline is
therefore the absolute `CLOCK_REALTIME` timespec supplied by the caller.

**Observed:** the static initializer writes only the signature and zeros. A
consumer may pass `PTHREAD_COND_INITIALIZER` to wait/signal without any
`pthread_cond_init` call, so the adapter cannot require its own initialization
tag. The implementation instead stores a host `CONDITION_VARIABLE` at offset 8
of the opaque area and treats signature `0x3CB0B1BB` as valid. This matches the
public initializer contract but is not Darwin's private structure.

## TL-2: C++ thread-local destruction contract

**Observed:** compiling `tests/fixtures/cxx_tls.cpp` with the pinned cross
toolchain (`clang++ -target x86_64-apple-macos10.13`, `lld`/`ld64.lld`) emits:

- TLV template symbols `_tracked$tlv$init` and `___tls_guard$tlv$init` in
  `__DATA,__thread_bss`.
- A `__DATA,__thread_vars` entry whose initializer relocations reference
  `__tlv_bootstrap` for both variables.
- An initializer `__tls_init` that checks/rearms `___tls_guard` and calls
  `__cxx_global_var_init`.
- `__cxx_global_var_init` performs the dynamic initialization and then calls
  `__tlv_atexit` with **three** arguments: destructor pointer, object pointer
  and `&___dso_handle`.
- Undefined Mach-O imports `__tlv_atexit`, `__tlv_bootstrap` and
  `___dso_handle`.

The call site in the object is (unslid `__TEXT,__StaticInit` offsets):

```text
2b6: leaq    __ZN7TrackedD1Ev(%rip), %rdi
2bd: leaq    ___dso_handle(%rip), %rdx
2c4: callq   __tlv_atexit
```

with `RSI` already holding the initialized object address. **Inference:**
`___dso_handle` is expected to resolve to a valid address; ld64 synthesizes a
local definition for it, so adding a second definition in the System stub
library causes a duplicate-symbol failure. The adapter supplies a single stable
symbol and accepts the third argument but does not use it to filter DSOs.

**Unknown:** whether other compilers or deployment targets call
`__cxa_thread_atexit` instead, and the exact Darwin ordering between TLV
destructors, pthread key destructors, `atexit` callbacks and image finalizers.
The current implementation runs TLV destructors at thread exit and at main
session teardown, but does not claim Darwin's ordering.

## Boundaries

- Header evidence is pinned to one Xcode build; SDK compatibility with other
  deployment targets is not established.
- The opaque bytes beyond `__sig` are not decoded. The adapter's host pointer
  placement is an implementation choice, not an Apple-internal layout.
- `PTHREAD_PROCESS_SHARED` is accepted and reported by the attribute object but
  no process-shared mapping or cross-process wakeup is implemented.
- Cancellation, cleanup handlers, `pthread_cond_signal_thread_np` and relative
  timedwait are not implemented.
- No reference-Mac run establishes runtime equivalence for any of these
  contracts.
