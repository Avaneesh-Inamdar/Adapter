# ARC and zeroing weak ownership

Recorded 2026-10-22. Adapter executes actual Clang ARC lowering for runtime-owned,
raw-isa Intel objects. This is an ownership/runtime slice; it does not provide
Foundation or make Xcode executable.

## Implemented and exercised

- Strong assignment and compiler-generated cleanup of live object ivars,
  including a subclass and superclass each owning the same child.
- Weak initialization, assignment, retained loads, copy/move, destruction and
  zeroing. A final-release race cannot resurrect a dying object. `OrNil` accepts
  a dying target by storing nil.
- Guest `retain`/`release`/`autorelease` overrides and root libobjc helpers.
  Weak callbacks run outside the registry lock while an internal pin protects
  their object.
- Explicit nested pools, reverse draining and objects autoreleased by a
  destructor into the currently draining pool.
- Retained bridge transfers and recognized x64 return-value handoffs, including
  unsafe claims. Unrecognized instruction patterns take the ordinary
  autorelease/retain fallback.
- Executable `.cxx_destruct` methods, subclass first. Strong/weak bitmap strings
  are validated before constructors and do not cause a second cleanup pass.

The implementation is [ownership.rs](../src/objc/ownership.rs), with small return
thunks in [dispatch.s](../src/objc/dispatch.s). [Contract v4](abi/objc-abi.md)
records exact entry points, layouts, limits, callback and failure rules. Definitions
are pinned to Apple's
[NSObject.mm](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/NSObject.mm),
[weak implementation](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-weak.mm)
and [Clang's ARC ABI](https://clang.llvm.org/docs/AutomaticReferenceCounting.html).
No Apple implementation was copied into Adapter.

## Consumers and regressions

[objc_arc.m](../tests/fixtures/objc_arc.m) is built twice: an MRC root that uses
libobjc's root helpers, then an ARC consumer at `-O0` or `-O2`. The root provides
the RR and weak methods that a free-standing root class needs on macOS. The ARC
part contains strong/weak ivars and properties, generated destructors, pools and
bridges. It is not a hand-written simulation of compiler ownership calls.

The local fixtures run both optimization levels with classic, chained and
relative fixups. [Six integration methods](../tests/integration/test_arc.py)
cover the positive variants plus rejection of constructor-only flags,
overlapping/out-of-bounds ownership maps, missing cleanup and object ivars that
overlap isa. Rejections happen before `objc:ARC constructor` is printed.

At the ARC checkpoint (`a6efb8c`), the full verifier passed **187 distinct tests**: 32 Rust, 30 Python unit/archive
and 125 integration. All 125 integration tests also passed against the release
build. The Mac comparisons below are additional evidence, not an increase in
that unit/integration count.

## Same-binary Mac reference

[Run 39811445457](https://github.com/Avaneesh-Inamdar/Adapter/actions/runs/39811445457)
used standard `macos-15-intel`, Darwin 24.6.0 x86_64 and Apple clang 17.0.0.
Each downloaded executable is executed unchanged by Adapter. The suites
`adapter-objc-arc-o0-v1` and `adapter-objc-arc-o2-v1` each contain 1,024 records:
128 repetitions of eight ownership families. They use a small fixed class graph
and vary the inherited-cleanup branch. The thread family synchronizes a
retained weak load across the creator's release, then checks zeroing after join. They are not thousands of independent class layouts.

Both suites match **1,024/1,024 in both debug and release**. The suite keeps raw
stdout, identities, exit status and parsed records. Threaded weak tests serialize
only invariants after join, not scheduler-dependent counts of successful loads.

An initial native build added an unused Foundation dependency through Clang's
ARC link driver. The build recipe now compiles ARC and MRC objects separately
and links those objects with libobjc. The retained reference executable is that
linked result; no downloaded binary was patched to remove a dependency.

## Remaining boundaries

ARC Blocks ownership, foreign/externally allocated objects, atomic/copy
properties, associations, Objective-C exceptions and ARC exception cleanup are
not supplied by this slice. Arbitrary custom RR/weak policies and additional
compiler return sequences need more reference probes. Dynamic Objective-C
registration, shared-cache metadata, tagged/packed isa, Swift and Foundation
remain separate P1 work. Autorelease without an explicit pool is unsupported.

## Retained evidence

Workflow commit: `0590119`. One hashed source supplies the MRC root and ARC consumer.

| Compiler level | Executable SHA-256 | Source SHA-256 |
| --- | --- | --- |
| `O0` | `dde1c7f1e0d64d9ae65027657f74a700c74c7211bf8abbbd95a26f5de38a23b6` | `8971d2ed705d408ecb11ab928bc12f2549b8750368fe7b29705efc4f5090ffb0` |
| `O2` | `e54173483b3ef8c19ad882e6af3ab2525b0581d54c5a1189db43b589a9a47d41` | `8971d2ed705d408ecb11ab928bc12f2549b8750368fe7b29705efc4f5090ffb0` |

| Capture/report | SHA-256 of stored bytes |
| --- | --- |
| [test-results/mac-reference-objc-arc-o0-39811445457.json.gz](test-results/mac-reference-objc-arc-o0-39811445457.json.gz) | `c67f3dd4fe93c0f61cbb3c372c046d2f3f48c44736cf0192bf815de4fb566725` |
| [test-results/differential-objc-arc-o0-candidate-debug.json.gz](test-results/differential-objc-arc-o0-candidate-debug.json.gz) | `bdc60da824108e44ec89d1f20b43c4c9926e5f1550c59104d08fcb43c62454c0` |
| [test-results/differential-objc-arc-o0-debug.json](test-results/differential-objc-arc-o0-debug.json) | `44e67c759c6c206cfb87615e7338d3d95bcee4ab0be18fa476460ac8f94d44ca` |
| [test-results/differential-objc-arc-o0-candidate-release.json.gz](test-results/differential-objc-arc-o0-candidate-release.json.gz) | `57cf95dc755d93c960197c02387407cb485265d8f77e634f68ce644e82687a9b` |
| [test-results/differential-objc-arc-o0-release.json](test-results/differential-objc-arc-o0-release.json) | `4748a0c5e3e925f6e147ef17b3da7b245a36a4556bac7a96f876c43e314999a5` |
| [test-results/mac-reference-objc-arc-o2-39811445457.json.gz](test-results/mac-reference-objc-arc-o2-39811445457.json.gz) | `a1096e0929a88b7a97948beacddd9092100887ad5494e1a706ce575a9f0a983d` |
| [test-results/differential-objc-arc-o2-candidate-debug.json.gz](test-results/differential-objc-arc-o2-candidate-debug.json.gz) | `6ec34b51e0534f0d3ba034228c2ce680c806e509b495b010d972977b77a03b61` |
| [test-results/differential-objc-arc-o2-debug.json](test-results/differential-objc-arc-o2-debug.json) | `df1223f1bf72a96c8affd67c0552a1c2f441621a2e17a93fe177b5d0aad27292` |
| [test-results/differential-objc-arc-o2-candidate-release.json.gz](test-results/differential-objc-arc-o2-candidate-release.json.gz) | `616c3bb6e06ddb12e2a7c1e6c3fd8da95c49af1f7907bc0e79d1803ca2e5720e` |
| [test-results/differential-objc-arc-o2-release.json](test-results/differential-objc-arc-o2-release.json) | `bf314b11ac52ae769d6d69c71516db19e16a20e159633c6f36e36d4c98e65990` |

The gzip files retain complete reference/candidate captures in Git. Decode them before using `tools/differential.py compare`. The workflow artifact additionally retains the exact executables, source, objects and build manifests for 14 days.
