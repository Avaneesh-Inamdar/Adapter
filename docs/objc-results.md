# Objective-C slice results

Recorded 2026-10-22 on Windows x86-64. This slice loads compiler-emitted Intel Objective-C metadata and dispatches guest IMPs. It does not load Foundation or establish Xcode GUI compatibility.

## Implemented

- Raw class/metaclass registration, absolute method lists, bounded scalar ivar layout checks and canonical selectors.
- Instance and class lookup, inherited methods and `objc_msgSendSuper2`.
- Superclass-first own `+load`, lazy `+initialize`, typed GPR/SSE/stack argument preservation and nil scalar returns.
- Runtime-owned object allocation/disposal, metadata introspection, guest `alloc`/`init` dispatch and explicit balanced ownership/pools.
- Rejection of unsupported metadata before guest load methods or constructors. Failed registration does not partially patch selectors.

The exact admission rules, imports and remaining exclusions are in [Objective-C ABI contract v1](abi/objc-v1.md). Source evidence and compiler-output hashes are in [the pinned evidence report](reverse-engineering/objc-runtime-evidence.md).

## Confirmed checks

| Check | Result |
| --- | --- |
| `tests/fixtures/objc.m` through Adapter | Class method resolved from metadata; process returned 42 |
| `objc_core`, classic fixups | Passed: constructor marker, load/initialize order, inheritance, class/instance super sends, object and selector APIs, integer/float/stack arguments, nil and explicit balanced ownership |
| `objc_core-chained`, absolute methods | Same successful consumer result; linker disables relative method-list rewriting for this positive case |
| `tests/integration/test_objc.py` | Five test methods confirmed passing: one positive method exercises both fixup forms; four negative methods reject categories, relative lists, excessive method counts and unsupported image flags |
| Negative constructor checks | No constructor marker or other guest stdout before rejection; status 125 |
| `src/objc/mod.rs` unit test | Passed native Windows dispatch and atomic registration checks, including invalid IMP and ARC flags |
| `cargo clippy --locked --all-targets -- -D warnings` during slice development | Passed |

The positive consumer prints:

```text
objc:constructor
objc:metadata, inheritance, initialize, GP/SSE/stack, nil and ownership passed
```

The consumer source is [objc_core.m](../tests/fixtures/objc_core.m); the assertions are in [test_objc.py](../tests/integration/test_objc.py). The extra constructor marker makes rejection ordering observable rather than inferred from runtime logs.

## Pending validation and implementation

The final full verification run passed all 16 stages with **160 distinct tests: 32 Rust, 22 archive and 106 integration**; all 106 integration tests also passed against the release build, and the five Objective-C tests are in both profiles. Focused passes above are not a substitute for that run.

No Objective-C reference process has been executed on a real Mac. The compiled metadata evidence uses LLVM on Windows targeting macOS. Darwin equivalence, concurrent initialization, selector uniqueness across multiple startup images and broader real application behavior remain unverified.

The next runtime work is relative method lists, categories/protocols and stronger ownership/lifecycle coverage, with a separate ABI contract revision and consumers for each. Foundation and AppKit require additional subsystems. Dynamic Objective-C plugins, shared-cache metadata, ARC/weak, exceptions, Swift, packed/tagged isa and aggregate/x87 message returns remain unsupported; the contract lists the precise boundaries.
