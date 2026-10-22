# Objective-C ABI contract v4

Status: implemented bounded Intel slice. Owner: `src/objc/mod.rs` and `src/objc/dispatch.s`. Loader integration: `src/runtime/mod.rs`, `src/loader/link.rs` and `src/inspect/mod.rs`.

This contract describes Adapter's admitted subset, not the complete Apple Objective-C ABI. A subsystem change that expands the admitted metadata, import behavior or calling convention must update this contract and add a consumer or rejection test. Implementations must not replace an unsupported operation with a success no-op.

Revision 2 adds relative ("small") method lists and startup categories. Revision 1 admitted only 24-byte absolute method lists and rejected every `__objc_catlist` image.
Revision 3 admits compiler-emitted protocols, class/category adoption and property
records, and adds ten read-only introspection entry points. It leaves protocol
objects' on-disk isa unchanged; sending Objective-C messages to a protocol object
is unsupported.

Revision 4 adds runtime-owned ARC strong/weak ownership, guest RR dispatch, compiler ivar cleanup and x64 return-value ownership adapters. Its root-class helpers are libobjc adapters, not Foundation's NSObject implementation.

## Target and evidence

The target is little-endian Darwin x86-64 LP64: pointers and `long` are 8 bytes. Windows host C `long` is not interchangeable. Guest methods use SysV64; Windows x64 is only the host calling convention.

The layout evidence is pinned to Apple objc4 commit [`fb265098298302243cd7eeaa1f63f0ba7786dd9a`](https://github.com/apple-oss-distributions/objc4/tree/fb265098298302243cd7eeaa1f63f0ba7786dd9a) and LLVM 23.1.3 compiler outputs. Commands, hashes, source links and observed limitations are in [the evidence report](../reverse-engineering/objc-runtime-evidence.md). These source and binary observations are separate from runtime verification on macOS. A hosted Intel Mac has since matched all 4,096 cases of the same-binary `adapter-objc-v1` differential suite in debug and release, which is the reference evidence for the dispatch, category and `+load` behavior below; the macOS recorded run and its retained captures are in [differential testing](../differential-testing.md#recorded-objective-c-run-2026-10-22).

## Admitted records

All addresses below are relocated guest addresses. Counts, ranges and strings are checked before dereference. Names passed to a public entry point (`sel_registerName`, `sel_getUid`, `objc_getClass`) may live in a mapped metadata segment **or in the calling thread's stack frame**, so the runtime admits that bounded window for its one-byte string reads; it does not accept arbitrary guest addresses. `class_getInstanceSize` answers `ro.instanceSize` unchanged, so a compiler-emitted root class with no ivars reports 0 as Darwin does, while allocation still adds the isa word and the runtime's own minimum. The runtime keeps its state in host-owned maps; it does not rewrite a disk `class_ro_t` pointer into a fabricated Apple `class_rw_t`.

| Record | Byte offsets and fields | Size |
| --- | --- | ---: |
| Class | `isa:ptr@0`, `superclass:ptr@8`, cache storage at 16, `data:ptr@32` | 40 |
| Class read-only record | `flags:u32@0`, `instanceStart:u32@4`, `instanceSize:u32@8`, reserved at 12, `ivarLayout:ptr@16`, `name:ptr@24`, methods at 32, protocols at 40, ivars at 48, weak layout at 56, properties at 64 | 72 |
| Method-list header | `entsizeAndFlags:u32@0`, `count:u32@4`; entries at 8 | 8 |
| Absolute method | selector-name pointer at 0, type-string pointer at 8, IMP at 16 | 24 |
| Relative method | selector-slot offset `i32@0`, types offset `i32@4`, IMP offset `i32@8`; each offset is relative to its own field address | 12 |
| Category | `name:ptr@0`, `cls:ptr@8`, instance methods at 16, class methods at 24, protocols at 32, instance properties at 40; class properties at 48 when image-info bit 6 is set | 48 / 56 |
| Protocol | isa at 0, name at 8, inherited protocols at 16, required instance/class methods at 24/32, optional instance/class methods at 40/48, instance properties at 56, size/flags at 64/68; optional extended types at 72, demangled name at 80, class properties at 88 | 72 / 80 / 88 / 96 |
| Protocol adoption list | pointer-sized count at 0, protocol pointers at 8 | 8 + count × 8 |
| Protocol method | name at 0, type encoding at 8, null implementation at 16; list header has entry size 24 and 32-bit count | 24 |
| Property | name at 0, attributes at 8; list header has entry size 16 and 32-bit count | 16 |
| Method-description result | canonical selector at 0, type string at 8; SysV64 returns the two pointers in RAX/RDX | 16 |
| Ivar-list header | `entsize:u32@0`, `count:u32@4`; entries at 8 | 8 |
| Ivar | offset-variable pointer at 0, name pointer at 8, type pointer at 16, alignment at 24, size at 28 | 32 |
| Image info | `version:u32@0`, `flags:u32@4` | 8 |
| Super2 send record | original receiver at 0, **current** class or metaclass at 8 | 16 |

Admission rules:

- Class data must be an aligned, unflagged pointer to the read-only record. Admitted flags are `RO_META=1`, `RO_ROOT=2`, `RO_HIDDEN=16`, `RO_IS_ARC=128`, `RO_HAS_WEAK_WITHOUT_ARC=512`, and the pair `RO_HAS_CXX_STRUCTORS=4` / `RO_HAS_CXX_DTOR_ONLY=256`. Either destructor bit without the other is rejected: C++ construction remains unsupported. An ordinary destructor-marked class must define its own executable `.cxx_destruct` method.
- Image-info version must be 0; flags must be 0 or `0x40`. Bit `0x40` describes the category-properties ABI and does not mean ARC is enabled.
- A method-list header is either `entsize == 24` with no flags, or the small flag `0x80000000` with `entsize == 12`. The direct-selector flag `0x40000000` and any other entry size are rejected. Ivar entry size must equal 32. Ivar offset variables are read as low 32-bit offsets; each field must fit the compiler's instance size. Object ivars must be 8 bytes and cannot overlap isa. Strong/weak bitmap strings are validated as described below.
- Class names must be nonempty and unique. Class/metaclass links must agree, ordinary superclass chains must be acyclic, and subclass instance sizes cannot shrink below their superclass.
- A root metaclass's superclass is its ordinary root class and its isa is itself. A child metaclass's superclass is its ordinary superclass's metaclass; its isa is the root metaclass.
- Outside the dyld shared cache a relative selector field names a `__objc_selrefs` slot, so the name pointer is read from that slot; relative type and IMP fields are direct field-relative addresses. An entry whose points land outside a mapped segment, or whose IMP is not executable, is rejected before registration.
- Every method IMP must lie in an executable mapped region. Metadata and string pointers must lie in readable mapped regions; selector slots must be writable under the final segment protections.
- A category must have a nonempty name and a registered ordinary class, and may appear after that class in link order. Category instance methods/properties attach to the class and category class methods/properties to its metaclass. Adopted protocols attach at both levels. Category metadata replaces the class's own matching method/property; two categories supplying the same method or property at one level are rejected as ambiguous.
- Compiler protocol records require null isa, zero flags, a nonempty unique name and an acyclic inheritance graph. Duplicate names at different addresses are rejected rather than silently coalesced. The optional demangled-name field must be null. All protocol method implementations must be null. Extended type arrays are bounds/string validated but are not used as the method-description API's type encoding.
- Property names and attribute strings must be nonempty; duplicate names inside one list are rejected. Attributes describe compiler metadata only: this does not implement atomic access, object ownership or synthesized accessors. The latter execute as compiled guest methods.
- The linker may leave a category that carries `+load` in both `__objc_catlist` and `__objc_nlcatlist`; each record is attached once, keyed by its address.
- Limits: 8,192 total class/metaclass records; recursive class-graph depth bounded at 128 edges; 8,192 entries per reference section; 65,536 entries per method or ivar list; strings must terminate within 4,096 bytes; compiler instance size at most 1 MiB.

The protocol slice additionally caps definitions at 8,192, inheritance depth at
128, any adoption list at 8,192 entries, total parsed adoption references and
protocol methods at 65,536 each, and property records at 65,536. Repeatedly parsed
adoption lists count toward the reference budget.

Admitted section names are `__objc_imageinfo`, `__objc_classlist`, `__objc_nlclslist`, `__objc_selrefs`, `__objc_classrefs`, `__objc_superrefs`, `__objc_catlist`, `__objc_nlcatlist`, `__objc_protolist`, `__objc_protorefs`, `__objc_const`, `__objc_data`, `__objc_ivar`, `__objc_methlist`, `__objc_methname`, `__objc_methtype` and `__objc_classname`. Recognizing a section name does not bypass validation of its records. Other Objective-C metadata is rejected.

## Registration and lifecycle

The loader maps and fixes up the complete startup image graph, validates guest initializer/finalizer targets and applies final segment protections. It then installs the guest runtime guards and calls `objc::begin` before any guest Objective-C method or C initializer.

`begin` validates the whole supplied graph, reads every class and category method list, collects class/super references and prepares canonical selector values. It attaches category methods and patches selector slots only after every validation succeeds, then publishes the session. A rejected registration must not leave a patched selector or an active partial registry. Class and super references retain their existing raw class pointers.

`Guard::initialize` directly invokes each class's own `+load` implementation, superclass first, and then every category `+load` with the target class as receiver. That matches Apple's `load_images` order, where all class `+load` calls precede category `+load` calls. Inherited `+load` is not called again for a subclass, and a category `+load` never replaces the class's own. This occurs before the loader's ordinary C constructors. A `+load` body can send a normal message and thereby trigger lazy initialization. Startup registration is fixed; reentrant loading of new Objective-C images is unsupported.

The first ordinary message initializes its class's superclass before the class. `+initialize` is looked up through the metaclass chain, so an inherited implementation can run with subclass `self`, and a category-provided `+initialize` takes the same priority as any other category class method. State is maintained per ordinary class: initializing with owner thread, or initialized. Same-thread recursion may proceed; competing threads wait on a condition variable. Registry locks are released before calling guest IMPs. These are implemented synchronization rules; a comprehensive concurrent Darwin differential test has not yet validated them.

The guard remains active through guest entry, owned-thread joins and guest finalizers. Guest thread cleanup drains explicit pools. Session teardown clears return-handoff/pool bookkeeping and frees remaining runtime-owned allocations without inventing guest deallocations for leaked objects or cycles. This is not Apple's complete process-exit behavior. Autorelease without an explicit pool remains unsupported.

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
| `objc_retain`, `objc_release`, `objc_autorelease` | Dispatch available guest RR methods, otherwise preserve the legacy runtime-owned count path; class objects are immortal and nil is harmless |
| `objc_autorelease`, `objc_autoreleasePoolPush`, `objc_autoreleasePoolPop` | Explicit per-thread pools; LIFO tokens and reverse release order |
| `objc_getProtocol`, `protocol_getName`, `protocol_isEqual` | Startup protocol lookup/identity; missing lookup is nil, nil name is `"nil"`, two nil protocols compare equal |
| `protocol_conformsToProtocol`, `class_conformsToProtocol` | Recursive protocol adoption; the C class API checks the specified class's adoption, without walking its superclasses |
| `protocol_getMethodDescription` | Required/optional and instance/class method descriptions, including inherited protocol declarations; missing result is two null pointers |
| `class_getProperty`, `protocol_getProperty` | Class lookup walks superclasses; protocol lookup walks adopted protocols. Protocol optional-property query returns nil as in the pinned runtime |
| `property_getName`, `property_getAttributes` | Borrowed pointers to validated guest metadata strings; nil property returns null |

Protocol/property handles and returned strings remain valid until image/session
teardown; callers must not free them. No new caller-owned allocation is returned
by these ten APIs. These queries do not trigger class `+initialize`. Unknown
non-null handles terminate with status 125; metadata admission remains atomic
with respect to selector patching and guest execution.

There can be at most 65,536 live runtime-owned allocations and 65,536 registered weak slots. Counts and allocation generations are host bookkeeping. Generation checks prevent a returning `-dealloc` from freeing a new object that reused its address. ARC ivar destruction and zeroing weak references are implemented below; exception cleanup remains unsupported.

## ARC and weak ownership

| Entry points (C spelling) | Contract |
| --- | --- |
| `_objc_rootRetain`, `_objc_rootRelease`, `_objc_rootAutorelease`, `_objc_rootRetainCount`, `_objc_rootTryRetain`, `_objc_rootIsDeallocating` | Root count/pool operations without recursive RR dispatch; try-retain fails once final release marks deallocating |
| `objc_storeStrong` | Retain incoming value, publish the slot, release old value; self-assignment is harmless |
| `objc_setProperty_nonatomic` | Strong store at the supplied instance offset; atomic/copy setters remain absent |
| `objc_initWeak`, `objc_storeWeak`, and `OrNil` variants | Register writable guest storage; pin while `allowsWeakReference` runs outside the registry lock. Normal variants reject a dying/denied value; `OrNil` variants store nil |
| `objc_loadWeakRetained` | Atomically load/pin a registered value and return +1; a dying value returns nil. An available `retainWeakReference` callback supplies the returned retain while an internal pin protects it |
| `objc_loadWeak` | Retained load followed by autorelease |
| `objc_copyWeak`, `objc_moveWeak`, `objc_destroyWeak` | Copy into uninitialized storage, move and nil the source, or unregister the slot; concurrent weak clearing is serialized |
| `objc_destructInstance` | Each class's own `.cxx_destruct`, subclass first, once; then weak clearing without freeing memory |
| `object_dispose` | Mark deallocating, destruct, zero remaining weak slots and free the runtime allocation |
| `objc_autoreleaseReturnValue`, `objc_retainAutoreleaseReturnValue`, `objc_retainAutoreleasedReturnValue`, `objc_claimAutoreleasedReturnValue`, `objc_unsafeClaimAutoreleasedReturnValue` | Matching x64 return handoff, with ordinary autorelease/retain fallback |
| `objc_retainAutorelease` | Retain followed by autorelease |

Strong/weak layout strings encode high-nibble skips and low-nibble scans in
pointer words starting at `instanceStart`. They must terminate within 4,096
bytes, stay inside the instance, avoid isa and not overlap each other. A nonempty
layout requires the ordinary class's destructor. The compiler's `.cxx_destruct`
performs field cleanup; the bitmap does not cause a second release pass.

Ownership slots must be aligned writable guest storage: mapped images,
runtime-owned instances, the calling thread's stack, or previously registered
weak storage whose lifetime the client still guarantees. Weak registration does
not keep the target alive. Final release sets count zero before `-dealloc`, so
concurrent weak loads cannot resurrect it. Guest RR, weak and destructor
callbacks run outside the registry lock. Client races mutating/destroying the
same weak variable remain unsupported.

Explicit pools drain backwards, including autoreleases added to the draining
pool by guest deallocation. The x64 return thunks capture the actual caller
return address and recognize `mov rax,rdi` followed by a relative or RIP-indirect
claim call, directly or through a validated Mach-O import stub. A thread-local
offer is accepted only for the matching object and claim return address. Unsafe
claim releases the offered +1. Other call sequences use ordinary autorelease
and retain fallback, which may extend temporary lifetimes compared with Apple's
optimized path; additional compiler patterns need probes.

The consumer `objc_arc.m` is compiled twice: an MRC root client of libobjc and an
actual ARC consumer at `-O0` or `-O2`. Both run with classic, chained and relative
fixups. Mac suites compare the identical downloaded executables. Tests cover
inherited strong/weak ivar cleanup, weak copy/move/zeroing, deallocating-object
`OrNil`, reentrant pool draining, return ownership, custom RR, bridges and a
weak-load/final-release race. Negative tests reject constructor-only flags,
overlapping/out-of-bounds layouts, missing cleanup and ivars overlapping isa
before the constructor marker.

## Failure behavior and exclusions

Unknown imports fail static linking/preflight. Unsupported or malformed Objective-C metadata fails worker registration before `+load`, C constructors or guest entry. Metadata validation requires mapped, relocated pointers, so `adapter-run --check` alone is not a complete Objective-C registration test.

Unknown selectors, tagged receivers, unknown/nonpointer isa, invalid runtime-owned-object operations and invalid pool order produce an Objective-C diagnostic and terminate the worker with status 125. There is no message forwarding or method-resolution fallback.

Unsupported in v4: foreign/externally allocated objects, ARC Blocks ownership adapters, atomic/copy properties, dynamic Objective-C registration/unload, protocol messages/mutation/copy-list APIs, duplicate-protocol coalescing, direct-selector lists, shared-cache/preoptimized or signed metadata, GC, C++ constructors, exceptions/ARC exception cleanup, associations, dynamic class/method mutation, tagged/packed isa, Swift, fpret/stret/fp2ret and forwarding. Arbitrary custom RR/weak policies need further reference coverage. Foundation and AppKit remain separate work.

## Required checks

The consumers are `tests/fixtures/objc_core.m` and `tests/fixtures/objc_category.m`, built by `tests/build_fixtures.py` in classic, chained-fixup and relative-method-list forms. `objc_core` covers class/selector registration, inheritance, `+load`/`+initialize`, super sends, mixed GP/SSE/stack arguments, nil sends and explicit ownership. `objc_category` covers class and instance method overriding, a category-only class method, inheritance of category methods and class-before-category `+load` order. `tests/fixtures/objc.m` remains the minimal class-method return-42 probe.

`tests/integration/test_objc.py` runs all three fixup forms of both consumers and rejects excessive method counts and unsupported image flags without printing a fixture constructor marker. `tests/integration/test_runtime.py` contains the minimal positive probe. The unit test in `src/objc/mod.rs` checks atomic failure before selector patching, malformed IMP and Swift-flag rejection, and actual GPR/SSE/stack/nil dispatch.

`objc_metadata.m` adds protocol inheritance, category adoption, required/optional
descriptions, instance/class properties, inherited property lookup and scalar
guest accessors in all three fixup forms. Negative mutations reject oversized
or flagged protocols, adoption counts/cycles, nonnull description IMPs and
oversized property lists before its constructor marker. Its standalone
`adapter-objc-metadata-v1` form is compared against the same Mac-built executable.

Still needed: an explicit rejection test for a category that names an unregistered class; cross-image selector/protocol and inheritance consumers; concurrent/reentrant initialization stress; inherited-initialize and inherited-load negatives; invalid metadata pointers, graph/isa cases and additional type encodings; custom ownership boundary probes. New support requires positive ABI consumers and a retained rejection test for neighboring unsupported forms.
