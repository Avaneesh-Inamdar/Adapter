# Objective-C ABI contract v1

Status: implemented bounded Intel slice. Owner: `src/objc/mod.rs` and `src/objc/dispatch.s`. Loader integration: `src/runtime/mod.rs`, `src/loader/link.rs` and `src/inspect/mod.rs`.

This contract describes Adapter's admitted subset, not the complete Apple Objective-C ABI. A subsystem change that expands the admitted metadata, import behavior or calling convention must update this contract and add a consumer or rejection test. Implementations must not replace an unsupported operation with a success no-op.

## Target and evidence

The target is little-endian Darwin x86-64 LP64: pointers and `long` are 8 bytes. Windows host C `long` is not interchangeable. Guest methods use SysV64; Windows x64 is only the host calling convention.

The layout evidence is pinned to Apple objc4 commit [`fb265098298302243cd7eeaa1f63f0ba7786dd9a`](https://github.com/apple-oss-distributions/objc4/tree/fb265098298302243cd7eeaa1f63f0ba7786dd9a) and LLVM 23.1.3 compiler outputs. Commands, hashes, source links and observed limitations are in [the evidence report](../reverse-engineering/objc-runtime-evidence.md). These source and binary observations are separate from runtime verification on macOS; no real Mac reference result has been collected for this slice.

## Admitted records

All addresses below are relocated guest addresses. Counts, ranges and strings are checked before dereference. The runtime keeps its state in host-owned maps; it does not rewrite a disk `class_ro_t` pointer into a fabricated Apple `class_rw_t`.

| Record | Byte offsets and fields | Size |
| --- | --- | ---: |
| Class | `isa:ptr@0`, `superclass:ptr@8`, cache storage at 16, `data:ptr@32` | 40 |
| Class read-only record | `flags:u32@0`, `instanceStart:u32@4`, `instanceSize:u32@8`, reserved at 12, `ivarLayout:ptr@16`, `name:ptr@24`, methods at 32, protocols at 40, ivars at 48, weak layout at 56, properties at 64 | 72 |
| Absolute method-list header | `entsizeAndFlags:u32@0`, `count:u32@4`; entries at 8 | 8 |
| Absolute method | selector-name pointer at 0, type-string pointer at 8, IMP at 16 | 24 |
| Ivar-list header | `entsize:u32@0`, `count:u32@4`; entries at 8 | 8 |
| Ivar | offset-variable pointer at 0, name pointer at 8, type pointer at 16, alignment at 24, size at 28 | 32 |
| Image info | `version:u32@0`, `flags:u32@4` | 8 |
| Super2 send record | original receiver at 0, **current** class or metaclass at 8 | 16 |

Admission rules:

- Class data must be an aligned, unflagged pointer to the read-only record. Only class flags `RO_META=1`, `RO_ROOT=2` and `RO_HIDDEN=16` are admitted.
- Image-info version must be 0; flags must be 0 or `0x40`. Bit `0x40` describes the category-properties ABI and does not mean ARC is enabled. Actual categories remain unsupported.
- Method entry size/flags must equal 24 exactly. Ivar entry size must equal 32. Ivar offset variables are read as low 32-bit offsets; each field must fit the compiler's instance size. Direct object ivars and nonnull ownership layouts are rejected.
- Class names must be nonempty and unique. Class/metaclass links must agree, ordinary superclass chains must be acyclic, and subclass instance sizes cannot shrink below their superclass.
- A root metaclass's superclass is its ordinary root class and its isa is itself. A child metaclass's superclass is its ordinary superclass's metaclass; its isa is the root metaclass.
- Every method IMP must lie in an executable mapped region. Metadata and string pointers must lie in readable mapped regions; selector slots must be writable under the final segment protections.
- Limits: 8,192 total class/metaclass records; recursive class-graph depth bounded at 128 edges; 8,192 entries per reference section; 65,536 entries per method or ivar list; strings must terminate within 4,096 bytes; compiler instance size at most 1 MiB.

Admitted section names are `__objc_imageinfo`, `__objc_classlist`, `__objc_nlclslist`, `__objc_selrefs`, `__objc_classrefs`, `__objc_superrefs`, `__objc_const`, `__objc_data`, `__objc_ivar`, `__objc_methname`, `__objc_methtype` and `__objc_classname`. Recognizing a section name does not bypass validation of its records. Other Objective-C metadata is rejected.

## Registration and lifecycle

The loader maps and fixes up the complete startup image graph, validates guest initializer/finalizer targets and applies final segment protections. It then installs the guest runtime guards and calls `objc::begin` before any guest Objective-C method or C initializer.

`begin` validates the whole supplied graph, collects class/super references and prepares canonical selector values. It patches selector slots only after every validation succeeds, then publishes the session. A rejected registration must not leave a patched selector or an active partial registry. Class and super references retain their existing raw class pointers.

`Guard::initialize` directly invokes each class's own `+load` implementation, superclass first. Inherited `+load` is not called again for a subclass. This occurs before the loader's ordinary C constructors. A `+load` body can send a normal message and thereby trigger lazy initialization. Startup registration is fixed; reentrant loading of new Objective-C images is unsupported.

The first ordinary message initializes its class's superclass before the class. `+initialize` is looked up through the metaclass chain, so an inherited implementation can run with subclass `self`. State is maintained per ordinary class: initializing with owner thread, or initialized. Same-thread recursion may proceed; competing threads wait on a condition variable. Registry locks are released before calling guest IMPs. These are implemented synchronization rules; a comprehensive concurrent Darwin differential test has not yet validated them.

The guard remains active through guest entry, owned-thread joins and guest finalizers. Session teardown clears the current thread's pool bookkeeping and frees remaining runtime-owned allocations without invoking guest deallocation methods. Applications must balance explicit pools and ownership before return; this is not Apple's complete process-exit or thread-exit autorelease behavior.

## Message calling convention

`objc_msgSend` receives `self` in RDI and selector in RSI. Explicit integer/pointer arguments continue through RDX, RCX, R8, R9 and the caller's stack. Floating-point arguments use XMM registers under SysV classification.

The dispatcher saves all six GPR argument registers, RAX/AL and XMM0 through XMM7 around lookup, preserves the caller's stack-argument positions, restores them, and tail-jumps to the resolved guest IMP. The lookup helper and any lazy initialization run without a registry lock held across guest execution. IMPs therefore receive the original typed argument layout and return directly to the original caller.

`objc_msgSendSuper2` unwraps the original receiver and starts lookup at `current_class->superclass`; a class-method super send supplies the current metaclass. Legacy `objc_msgSendSuper` is a different interface and is not exported.

Nil ordinary and Super2 receivers return zero in RAX, RDX, XMM0 and XMM1. Scalar integer, pointer, float and double returns are supported. Method type strings reject aggregate, complex, vector and long-double returns; vector argument encodings are also rejected. `stret`, `fpret`, `fp2ret` and forwarding imports are absent. There is no fabricated aggregate buffer or x87 return value.

## Supported imports

Names here use C spelling. Mach-O symbol-table names have an additional leading underscore. The adapter is selected for the exact install name `/usr/lib/libobjc.A.dylib`.

| Imports | Behavior |
| --- | --- |
| `objc_msgSend`, `objc_msgSendSuper2` | Validated lookup and typed tail dispatch described above |
| `_objc_empty_cache` | Runtime-owned static data; the current dispatcher does not use Apple cache contents |
| `sel_registerName`, `sel_getUid`, `sel_getName`, `sel_isEqual` | Session-wide canonical selector strings and equality |
| `objc_getClass`, `objc_lookUpClass` | Registered ordinary class lookup; missing name returns nil |
| `object_getClass` | Raw isa for a registered class or admitted instance; nil returns nil |
| `class_getSuperclass`, `class_getName`, `class_getInstanceSize`, `class_isMetaClass` | Registered metadata introspection; instance size is the compiler-reported size |
| `class_createInstance` | Zeroed runtime-owned allocation, raw isa, minimum storage 16 bytes, extra bytes bounded with total size at most 1 MiB |
| `object_dispose` | Frees a runtime-owned instance and returns nil |
| `objc_alloc` | Ordinary lazy initialization and guest `+alloc` dispatch; preserves a guest allocation override |
| `objc_alloc_init` | Guest `+alloc` followed by guest `-init`, nil-safe |
| `objc_retain`, `objc_release` | Separate reference counts for runtime-owned instances; class objects are immortal; nil is harmless |
| `objc_autorelease`, `objc_autoreleasePoolPush`, `objc_autoreleasePoolPop` | Explicit per-thread pools; LIFO tokens and reverse release order |

There can be at most 65,536 live runtime-owned allocations. Reference counts are Adapter bookkeeping, not a claim to implement custom `retain`/`release` overrides or ARC. Final release invokes an available `-dealloc`; it frees the instance if that method did not already call `object_dispose`. Autorelease requires a matching explicit pool. Instance ivar destruction, weak zeroing and exception cleanup are not supplied.

## Failure behavior and exclusions

Unknown imports fail static linking/preflight. Unsupported or malformed Objective-C metadata fails worker registration before `+load`, C constructors or guest entry. Metadata validation requires mapped, relocated pointers, so `adapter-run --check` alone is not a complete Objective-C registration test.

Unknown selectors, tagged receivers, unknown/nonpointer isa, invalid runtime-owned-object operations and invalid pool order produce an Objective-C diagnostic and terminate the worker with status 125. There is no message forwarding or method-resolution fallback.

Unsupported in v1: dynamic Objective-C `dlopen` registration/unload, categories, protocols, properties, relative/compact method lists, shared-cache/preoptimized metadata, signed class records, GC, ARC, weak references, custom retain/release semantics, ownership layouts, C++ construction/destruction, Objective-C exceptions, associations, dynamic class/method mutation, tagged pointers, packed/nonpointer isa, Swift classes/runtime, fpret/stret/fp2ret and forwarding. Foundation, AppKit and Apple framework compatibility are separate work.

## Required checks

The consumer is `tests/fixtures/objc_core.m`, built by `tests/build_fixtures.py` in classic and chained-fixup forms. Positive chained builds explicitly use `-no_objc_relative_method_lists`; a default relative-list variant is retained as a negative. `tests/fixtures/objc.m` is the minimal class-method return-42 probe. `tests/fixtures/objc_category.m` is a rejected category consumer.

`tests/integration/test_objc.py` checks classic/chained execution and rejects categories, relative methods, excessive method counts and unsupported image flags without printing the fixture's constructor marker. `tests/integration/test_runtime.py` contains the minimal positive probe. The unit test in `src/objc/mod.rs` checks atomic failure before selector patching, malformed IMP and ARC rejection, and actual GPR/SSE/stack/nil dispatch.

Still needed: cross-image selector and inheritance consumers; concurrent/reentrant initialization stress; inherited-initialize and inherited-load negatives; invalid metadata pointers, graph/isa cases and additional type encodings; custom ownership boundary probes; reference runs on a real Mac and differential comparison. New support requires positive ABI consumers and a retained rejection test for neighboring unsupported forms.
