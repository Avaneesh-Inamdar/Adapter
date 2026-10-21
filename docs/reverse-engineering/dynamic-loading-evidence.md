# Dynamic-loading ABI evidence

Investigated 2026-10-21. Scope: the pinned Xcode 27 SDK `dlfcn.h` and local
Apple-target compiler/linker output for a plugin image. No macOS reference run
was available, and the REA native provider on this Windows host accepts PE
targets only, so no Mach-O decompiler result is claimed. Finding IDs are local
report identifiers, not REA Evidence IDs.

## Artifact and method

- Input: `files/Xcode_27.xip`, 2,014,229,334 bytes, SHA-256
  `6a270c53a5a0c5e0ac78125342d44c3cfff2716ec390373cd5e30834d80a67c3`.
- SDK extracts: `target/artifacts/xcode27-sdk/`.
- Toolchain: LLVM 23.1.3 installed locally; `clang -target
  x86_64-apple-macos10.13` and `ld64.lld`. This is not Apple's shipping
  compiler, so the codegen observation below is evidence about this toolchain's
  output for the Apple target, not proof of Apple compiler behavior.

### dlfcn.h hardlink canary

The XRSimulator SDK copy at CPIO offset `7224987756` decodes as a 14,871-byte
NUL canary (`filesystem_placeholder: true`). The first member of the same XIP
hardlink group carries the real header:

| Field | Value |
| --- | --- |
| Member | `Xcode.app/Contents/Developer/Platforms/AppleTVOS.platform/Developer/SDKs/AppleTVOS.sdk/usr/include/dlfcn.h` |
| Reconstructed CPIO offset | `7224942596` |
| Size / SHA-256 | 14,871 / `01b3575f40973f5549655b9d44fb845b67c663fa201bdb1f3a0b0266e5ec046b` |
| Filesystem placeholder | false |

Its provenance record is `target/artifacts/xcode27-sdk/dlfcn.h.provenance.json`.
The placeholder is retained as `placeholders/dlfcn.h.canary` with its own
provenance so that a reader extracting only the XRSimulator path cannot silently
consume NUL bytes.

## DL-1: mode and special-handle constants

**Observed** from the canonical header (RTLD block at lines 303-321):

| Constant | Value |
| --- | ---: |
| `RTLD_LAZY` | `0x1` |
| `RTLD_NOW` | `0x2` |
| `RTLD_LOCAL` | `0x4` |
| `RTLD_GLOBAL` | `0x8` |
| `RTLD_NOLOAD` | `0x10` |
| `RTLD_NODELETE` | `0x80` |
| `RTLD_FIRST` | `0x100` |
| `RTLD_NEXT` | `(void *)-1` |
| `RTLD_DEFAULT` | `(void *)-2` |
| `RTLD_SELF` | `(void *)-3` |
| `RTLD_MAIN_ONLY` | `(void *)-5` |

The adapter pins the same values in `src/runtime/mod.rs` and asserts the four
special handles at compile time in `src/darwin/dl.rs`. A test also asserts that
`dlopen(NULL, ...)` returns the `RTLD_DEFAULT` value.

## DL-2: documented function contract

**Observed** from the header comments (lines 121-298), not from a running
system:

- `dlopen` recursively loads dependencies, runs static initializers and
  registers Objective-C classes before returning. A second call that resolves
  to the same path returns the existing handle and increments its reference
  count.
- A null path returns a handle equivalent to `RTLD_DEFAULT`.
- `RTLD_LOCAL` blocks symbols from `dlsym(RTLD_DEFAULT)`; `RTLD_GLOBAL` is the
  default; `RTLD_NOLOAD` returns an existing handle or fails; `RTLD_NODELETE`
  never unmaps; `RTLD_FIRST` stops handle lookup after the root image.
- `RTLD_LAZY` is documented as the default, but the header notes that lazy
  binding no longer exists with chained fixups, so `RTLD_NOW` is largely moot.
- A leaf-name argument is searched through `DYLD_LIBRARY_PATH`, caller/main
  `LC_RPATH`, the working directory and fallback library directories. A path
  containing a slash is tried through the leaf in `DYLD_LIBRARY_PATH` and then
  as supplied. `@rpath` is the documented way to use a dynamic run path.
- `dlsym` takes the C name even though the Mach-O symbol has a leading
  underscore, searches the supplied image and then the images it loaded unless
  `RTLD_FIRST` is set, and has the four special handles above.
- `dlerror` is per-thread, returns the last `dlopen`/`dlclose`/`dlsym` error,
  resets after the call, and returns null when no error occurred.
- `dlopen_preflight` and the `dladdr` family are separate declarations. They are
  not implemented by Adapter.

The header is not evidence that every documented search path is needed or
reachable in a sandboxed process. Adapter implements a consciously smaller
resolver and reports the unsupported arguments instead of approximating them.

## DL-3: destructor registration uses the image base

**Observed** in the linked fixture `tests/fixtures/generated/libplugin.dylib`
(built by `tests/build_fixtures.py`):

- The symbol table has a local `___GLOBAL_init_65535` at `__TEXT,__text` offset
  `0x540`; `__TEXT,__mod_init_func` references the exported constructor
  `_plugin_initialize`.
- Disassembling the linked initializer shows:

```text
540: pushq %rbp
541: movq  %rsp, %rbp
544: leaq  _plugin_terminate(%rip), %rdi
54b: xorl  %eax, %eax
54d: movl  %eax, %esi
54f: leaq  -0x556(%rip), %rdx
556: callq ___cxa_atexit
55b: popq  %rbp
55c: retq
```

The RIP-relative target at `0x54f` is computed from the next instruction
address `0x556`, so `%rdx` is `0`, the image's `__TEXT` base in this
non-slid dylib. In the object file the same operand is the undefined
`___dso_handle`; `ld64.lld` resolves it to the image base.

**Consequence for the adapter:** `__cxa_atexit` callbacks registered by a plugin
are tagged with the plugin image header, not with the main executable and not
with the adapter's own `___dso_handle`. `dlclose` must filter by each image's
header when it drains callbacks. Draining by the executable's handle or by the
global adapter handle would either run unrelated callbacks or none.

This is a local cross-link observation. Apple's shipping compiler and linker
were not available to confirm identical output; the adapter's behavior is
therefore tied to the observed contract, not asserted as universal compiler
behavior.

## DL-4: MH_BUNDLE and dynamic TLS behavior used by the fixtures

**Observed** by building fixtures with `ld64.lld`:

- `-dylib` produces filetype 6 images; `-bundle` produces filetype 8. Both can
  carry the same exports and initializer sections.
- `plugin_tls.c` compiles a `__thread` variable into a TLV template and a
  `__thread_vars` entry; the adapter rejects this plan before constructors run.
  This is a deliberate limitation of the first dynamic-loading slice, tested by
  the integration suite.

## Residual unknowns

- No reference-Mac execution of the fixtures, so constructor/destructor
  ordering, search-path behavior and `dlerror` text are unverified against
  Darwin.
- No Apple-shipped compiler/linker was available to compare `___dso_handle`
  lowering; only LLVM 23.1.3 output is measured.
- No real plugin consumer from a shipped Intel application has been executed.
- `RTLD_NEXT`, `RTLD_SELF`, `dladdr`, interposition, unmapping and dynamic TLS
  have no supporting evidence and are not claimed.
