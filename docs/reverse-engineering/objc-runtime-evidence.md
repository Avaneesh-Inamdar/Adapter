# Objective-C runtime slice: pinned ABI evidence

Investigated 2026-10-22 on Windows x86-64, starting at repository HEAD `5a9d6da1432f6ca1c20062e4d9a1503d365002cf`. Scope: compiler-emitted Intel Mach-O metadata, message ABI, lifecycle and explicit unsupported boundaries. REA native analysis remains unavailable on this host; LLVM was used. No Apple reference process was executed. Finding IDs below are local report labels, not REA Evidence IDs.

## Sources and actual experiments

Apple objc4 is pinned to [`fb265098298302243cd7eeaa1f63f0ba7786dd9a`](https://github.com/apple-oss-distributions/objc4/tree/fb265098298302243cd7eeaa1f63f0ba7786dd9a). XNU type context is pinned to [`f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/i386/_types.h`](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/i386/_types.h): target pointer/long sizes must follow Darwin LP64, not Windows LLP64. These sources are evidence/reference; no Apple runtime implementation was imported.

`C:\Program Files\LLVM\bin\clang.exe`, LLVM23.1.3 (`0d261d1ca552c95a8f007e061c787ac7132fbcbc`), successfully produced these analysis objects:

| Object under `target/analysis/objc-runtime-evidence/` | Consumer | SHA-256 |
| --- | --- | --- |
| `minimal.o` | Existing `tests/fixtures/objc.m`: root Probe, class method returning42 | `BB060343ABD2A2AEEDDDF19188D9878A75840FDE73BD983C0719001192955AD2` |
| `extended.o` | Analysis-only `extended.m`: root scalar ivars, subclass/super send, category, load/initialize, mixed/large-struct/long-double sends | `76B45C1FBF23D9C832BF6D790B44F88CE0F9054FA1195E4A5B9176565DA42C0B` |
| `arc.o` | Analysis-only `arc.m`: strong/weak ivars, weak local, autorelease pool | `EE438FF9FF2E8800978EE9ACF7A952C337417A928B341D3BDF53D99E64710129` |

Successful compile arguments: `clang.exe "--target=x86_64-apple-macosx14.0" "-fobjc-runtime=macosx-14.0" -fno-objc-arc -O0 -c INPUT -o OUTPUT`; replace `-c` with `-S` for matching `.s` outputs. ARC probe replaces `-fno-objc-arc` with `-fobjc-arc -fobjc-weak`. All probes are SDK-free, compiler-built fixtures, not independently macOS-built reference applications.

Inspection commands actually ran: `llvm-nm -n minimal.o`, `llvm-nm -u arc.o`, `llvm-readobj --relocations extended.o`, `llvm-objdump --macho --section-headers minimal.o`, and `llvm-objdump --macho --objc-meta-data minimal.o`. These are relocatable objects: symbolic assembly plus relocation records establish pointer targets. Some objdump zero-value annotations misleadingly name an unrelated symbol at address0; those annotations are not proof of a nonnull pointer.

## OBJC-1: ordinary class, ro and method records

**Observed, high confidence:** minimal.o has class at object VMA`0x50`, metaclass`0x78`, class method list`0xb8`, metaclass ro`0xd8`, class ro`0x120`. These are object VMAs, not executable addresses or file offsets. Layout agrees with the [pinned runtime header](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-runtime-new.h).

| Record | Fields: byte offset/type | Size |
| --- | --- | ---: |
| Compiler `class_t` / `objc_class` | isa0/ptr, superclass8/ptr, cache16/16bytes, data32/ptr+flags | 40 |
| `class_ro_t` | flags0/u32, instanceStart4/u32, instanceSize8/u32, reserved12/u32, ivarLayout16/ptr, name24/ptr, baseMethods32/ptr, baseProtocols40/ptr, ivars48/ptr, weakIvarLayout56/ptr, baseProperties64/ptr | 72 |
| Absolute method-list header | entsizeAndFlags0/u32, count4/u32; entries start8 | 8 |
| Absolute method entry | name0/ptr, types8/ptr, IMP16/ptr | 24 |
| Ivar-list header / entry | header entsize0/u32,count4/u32; entry offset-variable0/ptr,name8/ptr,type16/ptr,alignment24/u32,size28/u32 | 8 / 32 |

The minimal method list has header24/count1 and type string `i16@0:8`; metadata points to `+[Probe value]`. Root class flags2 (`RO_ROOT`), superclassNULL, isa=its metaclass. Root metaclass flags3 (`RO_ROOT|RO_META`), isa=self, superclass=root ordinary class. Metaclass instanceStart/Size are40. Root ordinary instanceStart/Size are0: a valid compiler output, not automatically malformed.

Extended MRC root has explicitly declared `void *isa` at0 and scalar Darwin long at8; instanceSize16, null strong/weak layouts. Ivar offset variables are emitted as qwords; Apple x86 runtime reads/writes their low32 bits. Child ordinary superclass=root; child meta superclass=root meta and isa=root meta. For allocation, upstream rounds instance size to a word boundary and enforces minimum16; do not allocate zero bytes merely because a root ro says0.

**Implementation decision inferred:** first admit ordinary aligned ro pointers, absolute24-byte methods, bounded counts/strings, and non-object primitive ivars with validated ranges. Reject Swift flags, ownership layouts, C++ structors and unsupported pointer encodings before registration. Keep runtime state separately rather than pretending disk ro is already realized `class_rw_t`. Every pointer must be checked after loader rebasing/binding, and every list needs checked `8 + count*stride` arithmetic.

## OBJC-2: selectors and class/super references

**Observed:** minimal `__objc_classrefs` at VMA`0x168` contains a relocation to Probe class; `__objc_selrefs` at`0x170` references `"value"`; `__objc_classlist` at`0x178` references the class definition. Main loads classref into RDI and selref into RSI, then calls `_objc_msgSend`. Definitions are `__objc_data`/`__objc_const`; strings use `__objc_classname`, `__objc_methname`, `__objc_methtype`.

Extended `+[EvidenceChild answer]` builds a 16-byte super record `{receiver=self, current_class=EvidenceChild metaclass}`. Its `__objc_superrefs` slot references the **current metaclass**, not the superclass. The dispatcher must start at `current_class->superclass` while leaving the original class object as receiver. This differs from public legacy `objc_msgSendSuper`, whose second record field is already the first class to search. [Public super contract](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/message.h), [Super2 implementation](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/Messengers.subproj/objc-msg-x86_64.s).

**Inference:** canonicalize selector names across method names and all selref slots, including multiple images; selector identity must not depend on independently allocated equal strings. Apple performs selector registration/uniquing in [objc-sel.mm](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-sel.mm). Check weak-import classrefs separately: an unresolved weak class is a null reference, not a class named by a guessed address.

## OBJC-3: message calling convention

**Compiler-observed:** an IMP receives self and selector before explicit arguments. Extended `callMixed` places selfRDI, selectorRSI, integer1RDX, double2.5XMM0, integer3RCX, integer4R8, integer5R9, integer6 on the stack. Code relocation offsets identify `_objc_msgSendSuper2` at text`0x10f`, ordinary `_objc_msgSend` at`0x154`/`0x19f`, `_objc_msgSend_stret` at`0x1e2`, `_objc_msgSend_fpret` at`0x228`.

A24-byte struct return uses hidden result pointerRDI, selfRSI, selectorRDX and `_objc_msgSend_stret`. Long double uses `_objc_msgSend_fpret`; float/double use ordinary send. The generated struct-return consumer checks nil and explicitly zeroes24bytes with memset.

The [Apple messenger](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/Messengers.subproj/objc-msg-x86_64.s) preserves argument state during lookup and tail-jumps to IMP. Normal nil returns clear RAX/RDX/XMM0/XMM1; fpret additionally supplies x87zero. Nil stret returns the destination pointer without clearing the buffer. Super2 loads current class from record+8, receiver from record+0, then follows superclass+8.

**Inference:** save/restore GPR arguments, RAX/AL where relevant to variadic sends, XMM0..7, stack arguments and required alignment around lookup. A generic C variadic wrapper cannot safely replace every typed Objective-C send. Scalar-only support must explicitly exclude stret/fpret/forwarding/exception cases rather than returning fabricated values. A missing method needs a controlled unsupported path until resolution/forwarding is implemented.

## OBJC-4: +load and +initialize

Extended `+load` creates a `__objc_nlclslist` entry for the root. `+initialize` is an ordinary class method in the metaclass method list; it creates no special nonlazy entry. The [load implementation](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-loadmethod.mm) invokes saved **own** load IMPs directly: superclass loads first, category loads after their parent class load; reentrant image loads are drained by the outer load pass. Do not dispatch inherited +load once per subclass or conflate it with initialize. A load body can itself send messages.

The [initialize implementation](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-initialize.mm) lazily initializes superclass before subclass, then sends `initialize` normally to the actual class. An inherited implementation can therefore run with subclass self. It tracks initializing/initialized state, permits same-thread reentrancy and blocks competing threads. **Inference:** initial tests need two classes, inherited initialize, repeated messages and recursive initialize; single-thread success alone cannot claim thread-safe initialization.

## OBJC-5: explicit feature boundaries

| Feature | Evidence / consequence for the first slice |
| --- | --- |
| Image flags | All three compiler probes emit `{version0,flags0x40}`, including ARC. [objc-abi.h](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-abi.h) identifies bits0 categories-optimized,1 supportsGC,2 requiresGC,3 shared-cache optimized,4 signed ro,5 simulated,6 category-class-properties,7 closure-optimized, Swift version fields above bit7. **Proposed safe admission:** version0 and flags0 or0x40; reject others until their representation is handled. This is a policy recommendation, not an upstream restriction. |
| Compact methods | Header defines small flag`0x80000000`, direct-selector flag`0x40000000`; small entries have three signed field-relative32bit displacements (12bytes). Outside shared cache the name displacement reaches a selref slot; in shared cache it can reach a selector directly. Runtime stride strips flagmask`0xffff0003`. Pointer-union/relative-list forms also exist. No compact list was produced by the installed default compiler probes. Absolute-only reader should reject these forms explicitly. |
| Raw/nonpointer isa | [isa.h](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/isa.h) x86 packed isa uses class mask`0x00007ffffffffff8`, magic mask`0x001f800000000001`, value`0x001d800000000001`. A real object's isa can contain ownership/weak/destructor bits; it is not always a pointer. Raw-isa-only support must enforce its object provenance. |
| Tagged receivers | [objc-internal.h](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-internal.h) uses low-bit tags for macOS x86-64 receivers, with tag tables/obfuscation. This differs from a real object's packed isa. Detect/reject tagged receivers **before** object memory access if unsupported. |
| Categories/protocols | Extended category emits `__objc_catlist`; category record contains seven pointer fields followed by compiler size64/padding. Modern upstream category tail fields are conditional. Categories add/override methods; protocol metadata/registration is distinct. Reject catlist/nlcatlist/protolist and nonnull protocol lists if not implemented; ignoring them changes lookup. |
| ARC/weak | ARC probe class ro flags`0x186`, meta`0x187`, instanceSize24, strong object+8, weak object+16. Strong layout is bytes`11 00`, weak`21 00`; compiler emits `.cxx_destruct`. Imageinfo still0x40, so it cannot distinguish ARC. Imports include storeStrong/storeWeak/initWeak/destroyWeak/loadWeakRetained/retainAutoreleaseReturnValue/autoreleaseReturnValue/poolPush/Pop. |
| Ownership semantics | [Clang ARC contract](https://clang.llvm.org/docs/AutomaticReferenceCounting.html) assigns retained results to alloc/new/copy/mutableCopy; init consumes self and returns retained. Strong assignment retains new before releasing old. Weak registration/load/zeroing must synchronize with deallocation; [Apple weak implementation](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-weak.mm) maintains referrer tables. These APIs cannot safely be success no-ops. Blocks, exceptions, associations and autorelease optimizations need separate consumers. |

RO bits relevant to rejection are observed in compiler output and corroborated by the runtime header: meta1, root2, C++ structors4, hidden0x10, exception0x20, Swift initializer0x40, ARC0x80, destructor-only0x100, weak-without-ARC0x200. `data` low Swift bits and runtime realized-state bits require interpretation rather than unconditionally treating data as ro. An ARC flag alone need not mean every method performs ownership operations, but a first MRC-only implementation may conservatively reject the image/class and report why.

## Actionable verification checklist and limits

1. Execute existing Probe fixture and verify return42; selector name/type/IMP must come from parsed metadata.
2. Add runtime consumers for raw-isa allocation/scalar ivar, inheritance, instance and class Super2, nil scalar send, mixed GPR/XMM/stack arguments, and selector equality across two images.
3. Verify own-load ordering independently of lazy/inherited initialize. Record events from consumer code, not only registration logs.
4. Add preflight negatives for ARC/weak layouts, compact lists, categories/protocols, Swift/signed/shared-cache metadata and tagged receivers. Fail before guest execution where possible.
5. Verify truncation, invalid pointers, count multiplication, malformed strings and graph cycles without following unchecked guest addresses.

**Implemented by this research:** only this tracked evidence report; generated source/object/assembly files live under ignored target/analysis. **Verified:** all three object and assembly compilations succeeded; LLVM metadata/relocation inspection matches the listed layouts and call forms. **Not verified:** Objective-C execution on DarwinBridge, real Apple-built application equivalence, message forwarding, exceptions, ARC/weak correctness, category/protocol integration, packed/tagged support or full concurrent initialization. No source/runtime changes or commits were made by this research task.

## Revision 2: relative method lists and startup categories

This section supersedes the "Compact methods" and "Categories/protocols" rows of OBJC-5 for the implemented slice: relative lists and startup categories are now admitted, direct-selector and shared-cache forms are still rejected.

Observed 2026-10-22 with the same LLVM 23.1.3 toolchain on Windows, by inspecting the generated fixtures with `target/debug/adapter-inspect.exe --json` and dumping raw section bytes.

| Fixture | Method-list container | `__objc_catlist` | Bytes | SHA-256 |
| --- | --- | --- | ---: | --- |
| `objc_core` (classic, `-no_fixup_chains`) | `__objc_const`, 24-byte absolute entries | absent | 19,936 | `ed5dbf9a211389733748ef467379a2f014b1a6fa4cd5764a2d2fbb34d57e42c4` |
| `objc_core-chained` (`-no_objc_relative_method_lists`) | `__objc_const` | absent | 23,976 | `e317a6cf75a42cd562c238fe72a437f6a73154b1f6b5ac327424bca1a1f9fe0a` |
| `objc_core-relative` (clang default relative lists) | `__objc_methlist`, 200 bytes | absent | 23,976 | `dd819e321b1e9620e104efb4aaa265829edf7d181dcdf262e4b4832b763d64ce` |
| `objc_category` (classic) | `__objc_const` | present, 8 bytes | 14,352 | `96aa0b5aa2c913d02b94d78bb0270dc40d727155952c2470fb30926ab12b374f` |
| `objc_category-chained` | `__objc_const` | present, 8 bytes | 18,432 | `b3dac2c9c952a9461504d2665fe07ef678fd0e06b3b55d33e892b3b04b0c85b4` |
| `objc_category-relative` | `__objc_methlist`, 160 bytes | present, 8 bytes | 18,432 | `f360ea9a11c079c723d5a8569a841b67766f1f7803aef8de414bb3aefa48e48c` |

With relative lists the linker moves every method list into `__objc_methlist`, and the class/category read-only records keep only pointers. The 12-byte entry's selector field names a `__objc_selrefs` slot rather than a string; following the field-relative offset into `__objc_selrefs` (112 bytes in `objc_core-relative`) is what makes the name read indirect.

`objc_category` carries a category `+load`. ld64.lld emitted the same record in both `__objc_catlist` and `__objc_nlcatlist`: each section is 8 bytes and holds the same pointer (`08 23 00 00 01 00 00 00` in the classic build). Apple keeps categories with `+load` only in the non-lazy list, so Adapter keys attachment by record address and attaches the category once.

Limits: these are Windows cross-compiled fixtures linked with ld64.lld, not outputs from Apple's linker, and no reference Mac executed them. The `+load` sequence (every class `+load`, then each category `+load`) is implemented from objc4 `load_images`/`prepare_load_methods` and still needs a Darwin differential probe.

## Revision 3: hosted Darwin differential

2026-10-22 (run `39673099968`, workflow commit `de9fbd5`, standard `macos-15-intel`): `tests/differential/objc_cases.m` was compiled on the Mac into `objc-differential` (14,824 bytes, SHA-256 `95ab88d498c5bbf706f50b3392eef44b47a3c4f5536b1de743b103679f0e4f60`) and captured natively, then captured again through `adapter-run.exe` from the identical bytes on Windows. All 4,096 records matched in debug and release; the retained captures and hashes are in [differential testing](../differential-testing.md#recorded-objective-c-run-2026-10-22).

Two assumptions were settled by that run rather than by reasoning:

1. `class_getInstanceSize` answers `ro.instanceSize` unchanged. Adapter had been changed to round a class up to one isa word, and the Darwin capture disagreed for a compiler-emitted root class with no ivars (0, not 8). The change was reverted; allocation keeps its own minimum, so an object still has room for its isa.
2. `objc_retain`/`objc_release` are **not** interchangeable with Apple's for a free-standing root class. On Darwin `objc_retain` sends `-retain`, which a compiler-emitted root class without ownership methods does not implement, so the first dispatch aborted with `unrecognized selector -[Probe retain]` after six families. Adapter's runtime-owned reference count and pool are therefore a documented divergence, not something the same-binary suite can probe with a plain root class; the probe now exercises object identity instead.

The `+load` order the implementation follows from objc4 (`load_images`/`prepare_method_load`) is now confirmed on Darwin: the encoded order code was `123` with three calls, so both class `+load` methods ran before the category `+load`, which ran with the target class as receiver. The category's class method replaced the class's own for a send through the class object.

Limits unchanged: this is a hosted macOS VM, the classes are compiler-emitted rather than Foundation classes, and ARC, weak tables, exceptions, tagged/packed isa and aggregate message returns remain unverified.
