# Objective-C slice results

Recorded 2026-10-22 on Windows x86-64. This slice loads compiler-emitted Intel Objective-C metadata and dispatches guest IMPs. It does not load Foundation or establish Xcode GUI compatibility.

## Implemented

- Raw class/metaclass registration, absolute and relative ("small") method lists, bounded scalar ivar layout checks and canonical selectors.
- Startup categories: class and instance methods attach to an existing registered class, category methods replace the class's own method of the same level, a selector supplied by two categories of one class is rejected, and a linker-duplicated `__objc_catlist`/`__objc_nlcatlist` record is attached once.
- Instance and class lookup, inherited methods and `objc_msgSendSuper2`.
- Superclass-first own `+load`, then category `+load` with the target class as receiver, lazy `+initialize`, typed GPR/SSE/stack argument preservation and nil scalar returns.
- Runtime-owned object allocation/disposal, metadata introspection, guest `alloc`/`init` dispatch and explicit balanced ownership/pools.
- Rejection of unsupported metadata before guest load methods or constructors. Failed registration does not partially patch selectors or attach a category.
- Compiler protocol definitions/adoption, inherited required/optional method descriptions,
  class/category/protocol properties and scalar compiler-generated accessors. Ten
  read-only introspection APIs return validated, session-lived metadata.

The exact admission rules, imports and remaining exclusions are in [Objective-C ABI contract v3](abi/objc-abi.md). Source evidence, compiler-output hashes and the relative-list observation are in [the pinned evidence report](reverse-engineering/objc-runtime-evidence.md). The new record layouts and query semantics are documented in [metadata evidence](reverse-engineering/objc-metadata-evidence.md).

## Confirmed checks

| Check | Result |
| --- | --- |
| `tests/fixtures/objc.m` through Adapter | Class method resolved from metadata; process returned 42 |
| `objc_core`, `objc_core-chained`, `objc_core-relative` | Passed: constructor marker, load/initialize order, inheritance, class/instance super sends, object and selector APIs, integer/float/stack arguments, nil and explicit balanced ownership. The classic build uses 24-byte absolute method lists; the chained build disables relative rewriting (`-no_objc_relative_method_lists`); the relative build keeps clang's 12-byte relative lists |
| `objc_category`, `objc_category-chained`, `objc_category-relative` | Passed in all three fixup forms: class and instance method overriding, a category-only class method, inherited category methods on a subclass, and `+load` order 1-3-2 (every class `+load` before the category `+load`) |
| `tests/integration/test_objc.py` | Ten test methods pass: three positive methods cover all three consumers across the three fixup forms; seven negative methods cover method counts, image flags, protocol size/flags/counts/cycles/nonnull IMPs and property counts |
| Negative constructor checks | No constructor marker or other guest stdout before rejection; status 125 |
| `src/objc/mod.rs` unit test | Passed native Windows dispatch and atomic registration checks, including invalid IMP and ARC flags |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| Same-binary Darwin differential, run 39673099968 | `adapter-objc-v1`: 4,096 cases, 0 mismatched in debug and release against one hosted Intel Mac capture ([details](differential-testing.md#recorded-objective-c-run-2026-10-22)) |
| Same-binary metadata differential, run 39805811658 | `adapter-objc-metadata-v1`: 4,096 comparisons, 0 mismatched in debug and release; covers a fixed compiler-emitted protocol/property graph, repeated queries and changing scalar accessor arguments ([details](differential-testing.md#recorded-objective-c-metadata-run-2026-10-22)) |

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

The full verification run passed all 16 stages with **181 distinct tests: 32 Rust, 30 archive and 119 integration**; every integration test also passed against the release build, and the Objective-C tests are in both profiles.

A hosted Intel Mac executed the same 4,096-case binary and matched Adapter in every case, so selector registration, dispatch, Super2 sends, category override, nil returns, class identity/size/metaclass state, object creation/identity and `+load` order now have Darwin reference evidence. That reference is a hosted macOS VM, and it covers compiler-emitted classes only. Concurrent initialization, selector uniqueness across multiple startup images, Foundation/AppKit behavior and broader real application behavior remain unverified.

The next runtime work is ARC/weak with its own Mac differential probe and real-library lifecycle coverage. Protocol/property mutation, copy-list APIs, protocol-object messages and cross-image duplicate-protocol coalescing remain unsupported. Foundation and AppKit require additional subsystems. Dynamic Objective-C plugins, shared-cache metadata, exceptions, Swift, packed/tagged isa and aggregate/x87 message returns remain unsupported; the contract lists the precise boundaries.
