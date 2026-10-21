# Objective-C slice results

Recorded 2026-10-22 on Windows x86-64. This slice loads compiler-emitted Intel Objective-C metadata and dispatches guest IMPs. It does not load Foundation or establish Xcode GUI compatibility.

## Implemented

- Raw class/metaclass registration, absolute and relative ("small") method lists, bounded scalar ivar layout checks and canonical selectors.
- Startup categories: class and instance methods attach to an existing registered class, category methods replace the class's own method of the same level, a selector supplied by two categories of one class is rejected, and a linker-duplicated `__objc_catlist`/`__objc_nlcatlist` record is attached once.
- Instance and class lookup, inherited methods and `objc_msgSendSuper2`.
- Superclass-first own `+load`, then category `+load` with the target class as receiver, lazy `+initialize`, typed GPR/SSE/stack argument preservation and nil scalar returns.
- Runtime-owned object allocation/disposal, metadata introspection, guest `alloc`/`init` dispatch and explicit balanced ownership/pools.
- Rejection of unsupported metadata before guest load methods or constructors. Failed registration does not partially patch selectors or attach a category.

The exact admission rules, imports and remaining exclusions are in [Objective-C ABI contract v2](abi/objc-abi.md). Source evidence, compiler-output hashes and the relative-list observation are in [the pinned evidence report](reverse-engineering/objc-runtime-evidence.md).

## Confirmed checks

| Check | Result |
| --- | --- |
| `tests/fixtures/objc.m` through Adapter | Class method resolved from metadata; process returned 42 |
| `objc_core`, `objc_core-chained`, `objc_core-relative` | Passed: constructor marker, load/initialize order, inheritance, class/instance super sends, object and selector APIs, integer/float/stack arguments, nil and explicit balanced ownership. The classic build uses 24-byte absolute method lists; the chained build disables relative rewriting (`-no_objc_relative_method_lists`); the relative build keeps clang's 12-byte relative lists |
| `objc_category`, `objc_category-chained`, `objc_category-relative` | Passed in all three fixup forms: class and instance method overriding, a category-only class method, inherited category methods on a subclass, and `+load` order 1-3-2 (every class `+load` before the category `+load`) |
| `tests/integration/test_objc.py` | Four test methods passing: two positive methods cover both consumers across the three fixup forms; two negative methods reject excessive method counts and unsupported image flags |
| Negative constructor checks | No constructor marker or other guest stdout before rejection; status 125 |
| `src/objc/mod.rs` unit test | Passed native Windows dispatch and atomic registration checks, including invalid IMP and ARC flags |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |

The positive consumers print:

```text
objc:constructor
objc:metadata, inheritance, initialize, GP/SSE/stack, nil and ownership passed
```

```text
objc:category methods override and extend the class
```

The consumer sources are [objc_core.m](../tests/fixtures/objc_core.m) and [objc_category.m](../tests/fixtures/objc_category.m); the assertions are in [test_objc.py](../tests/integration/test_objc.py). The constructor marker in `objc_core` makes rejection ordering observable rather than inferred from runtime logs.

## Verification and remaining work

The full verification run passed all 16 stages with **159 distinct tests: 32 Rust, 22 archive and 105 integration**; every integration test also passed against the release build, and the Objective-C tests are in both profiles.

No Objective-C reference process has been executed on a real Mac. The compiled metadata evidence uses LLVM on Windows targeting macOS. Darwin equivalence of category attachment order and relative-list decoding, concurrent initialization, selector uniqueness across multiple startup images and broader real application behavior remain unverified.

The next runtime work is protocols and properties, ARC/weak with a Mac differential probe, stronger ownership/lifecycle coverage and combined category-plus-load ordering on a reference Mac. Foundation and AppKit require additional subsystems. Dynamic Objective-C plugins, shared-cache metadata, exceptions, Swift, packed/tagged isa and aggregate/x87 message returns remain unsupported; the contract lists the precise boundaries.
