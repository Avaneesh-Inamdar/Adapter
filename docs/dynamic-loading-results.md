# Runtime dynamic-loading milestone — 2026-10-21

`dlopen`, `dlsym`, `dlclose` and `dlerror` now operate on Intel Mach-O images
loaded after startup. An unchanged Mach-O host loads classic, chained and
`MH_BUNDLE` plugins, calls exported code and data, observes per-image
constructors/destructors, and checks handle and error semantics. This is the D1
slice of the [OS services roadmap](roadmap.md), not a claim that Darwin's loader
is complete.

## Implementation

- The runtime snapshots every startup image's path, header, slide, public
  exports (absolute relocated addresses) and executable ranges. `dlsym` reads
  these snapshots and never borrows a loaded program after publication.
- `dlopen` refuses to run outside guest execution, resolves a path, and reuses
  the same library preparation, mapping, relocation, import binding and
  constructor path as startup. Classic and chained fixups and filetypes 6
  (`MH_DYLIB`) and 8 (`MH_BUNDLE`) are accepted; an executable root is rejected
  by the existing PIE/executable check.
- A canonical path that is already open returns the existing handle and
  increments its reference count. `RTLD_NOLOAD` returns that handle or fails.
  `RTLD_NOLOAD` does not populate a missing library.
- Default scope is global. `RTLD_LOCAL` hides a group from `RTLD_DEFAULT`.
  `RTLD_FIRST` limits handle lookup to the root image and excludes its adapter
  libraries. `RTLD_NODELETE` is accepted but has no distinct behavior because
  no image is ever unmapped. `RTLD_LAZY` and `RTLD_NOW` are equivalent here
  because all imports are bound eagerly before any constructor.
- `dlsym(handle, name)` prefixes `_` and searches the root, then the group's
  root-first dependency order, then its adapter libraries.
  `dlsym(RTLD_DEFAULT, name)` searches active global groups in load order.
  `dlsym(RTLD_MAIN_ONLY, name)` searches only the main executable's public
  exports. `RTLD_NEXT` and `RTLD_SELF` return an explicit unsupported error.
- `dlerror` is a per-thread slot returned exactly once; a later call returns
  null until another failure. NUL bytes in messages are escaped.
- Before a handle is published, the group's executable ranges are added to the
  exit-callback validator, so constructors can register `__cxa_atexit` entries
  and create guest threads. Constructors run after publication.
- `dlclose` decrements the reference count. At zero it marks the group
  inactive, drains that group's `__cxa_atexit` callbacks per image (the
  registered DSO is the image header), then runs image finalizers in reverse
  order. Mappings are deliberately retained; see the boundaries below.
- `_dyld_image_count`, `_dyld_get_image_header`, `_dyld_get_image_name` and
  `_dyld_get_image_vmaddr_slide` now include dynamically loaded images.
- Exact adapter install names (for example `libSystem.B.dylib`) can be passed to
  `dlopen`; they create an adapter-only group whose `dlsym` exposes that
  adapter library's registered symbols.

## Consumers and assertions

`tests/fixtures/plugin.c` is cross-compiled and linked three ways:
`libplugin.dylib` (classic fixups), `libplugin-chained.dylib` (chained fixups)
and `libplugin.bundle` (`MH_BUNDLE`). It exports `plugin_data = 41`,
`plugin_add`, `plugin_name` (`"adapter-plugin"`), and prints `plugin:init` /
`plugin:fini` from a constructor and destructor.

`tests/fixtures/dlopen.c` is the unchanged Mach-O host. For each variant it
checks:

- Loading is successful, `_dyld_image_count` grows by one, and `dlsym` finds a
  function, an `int` data export and a string-returning function.
- `dlopen` with `RTLD_NOLOAD` returns the same handle; two `dlclose` calls
  release the two references.
- A stale handle no longer resolves symbols and `dlerror` reports an error.
- `dlsym(RTLD_MAIN_ONLY, "host_marker")` finds the executable's own export;
  `plugin_add` is not visible through `RTLD_MAIN_ONLY` while the plugin is
  loaded.
- A missing symbol and a missing file both return null with a `"not found"`
  message, and each message is returned only once.
- `dlsym(RTLD_DEFAULT, "host_marker")` and `malloc` resolve through the startup
  and adapter image sets; `dlopen(NULL, ...)` equals `RTLD_DEFAULT`.
- A second load with `RTLD_LOCAL | RTLD_FIRST` can resolve its own
  `plugin_add`, cannot resolve dependency `malloc`, and is hidden from
  `RTLD_DEFAULT`.

Expected stdout for every variant:

```text
plugin:init
plugin:5:41:adapter-plugin
plugin:fini
plugin:init
plugin:fini
dl:passed
```

`tests/fixtures/plugin_tls.c` contains a `__thread` variable. Loading it fails
with `dynamic TLS is unsupported` before its constructor runs, proving the
rejection is a pre-constructor decision.

Two negative tests cover the loader boundary: `adapter-run --check` on a dylib
is rejected as a non-PIE executable (exit 125), and the TLS plugin host exits 91
with no constructor output.

## Verification

The full verifier passed all 14 stages. The current run contains **114 distinct
tests: 25 Rust, nine archive and 80 integration**; all 80 integration tests also
pass against the release build. The dynamic-loading test names are:

```text
test_dlopen_plugin_lifecycle_lookup_and_errors
test_dynamic_library_is_not_an_executable
test_dynamic_tls_plugin_rejected_before_constructors
```

`releasing mapped images: 1` in `--trace` output confirms that only the main
program's mapping is released at worker teardown; the three plugin loads in the
consumer remain mapped and their finalizers ran at `dlclose`. The existing jq
and ripgrep suites remain in the same run.

## Boundaries

- **No unmapping.** A guest may copy pointers, so `dlclose` only runs
  finalizers. Address space and code stay resident until the worker exits.
  This also means custom dlopen and `RTLD_NODELETE` have the same lifetime.
- **No dynamic TLS.** Images with TLV descriptors fail before constructors.
  TLS-bearing plugins require per-image TLV support in the dynamic registry.
- **No `@rpath` arguments.** `@rpath`, `@loader_path` and `@executable_path`
  passed to `dlopen` return an explicit unsupported error. Dependency install
  names still resolve those prefixes during the image's own graph preparation.
- **No cross-group sharing or aliasing.** A dylib already present as a
  dependency of another active group, or a dependency that overlaps one, is
  rejected rather than shared. Loading the same canonical path through a
  handle is supported and refcounted.
- **No `RTLD_NEXT`, `RTLD_SELF`, `dladdr`, interposition or ObjC registration.**
  Symbol lookup is group-local plus the default global scan; there is no
  caller-relative lookup or class registration.
- **No true symbol lazy binding.** All imports are bound eagerly, so
  `RTLD_LAZY` cannot defer failure to first call.
- **No cyclic initialization.** The startup graph rejects cycles; this slice
  inherits that behavior.
- **No reference-Mac comparison.** Output, error strings and ordering are
  Windows-side observations. See [ABI evidence](reverse-engineering/dynamic-loading-evidence.md).
- **No real application plugin consumer yet.** The pinned jq and ripgrep
  binaries do not call `dlopen`; the next D1 gate is a shipped Intel plugin.

## Reproduction

```powershell
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime.RuntimeTests.test_dlopen_plugin_lifecycle_lookup_and_errors
python -m unittest tests.integration.test_runtime.RuntimeTests.test_dynamic_library_is_not_an_executable
python -m unittest tests.integration.test_runtime.RuntimeTests.test_dynamic_tls_plugin_rejected_before_constructors
python tools/verify.py
```

The `dlfcn.h` provenance and the pinned RTLD values are recorded in
[the evidence](reverse-engineering/dynamic-loading-evidence.md). Fixtures are
generated under `tests/fixtures/generated/` and are not committed.
