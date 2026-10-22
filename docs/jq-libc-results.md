# jq and Darwin libc milestone — 2026-10-21

The unchanged jq 1.8.2 macOS amd64 executable now executes tested CLI workflows
on Windows x64. The previous `__DefaultRuneLocale` binding failure is resolved.
Work continued through its remaining libc imports into file/module workflows,
UTC dates, math and error behavior. No jq source was rebuilt into a Windows
replacement, and no Mac/VM executes the guest code.

## Implementation

- Exact LP64 RuneLocale layout and C/POSIX classification data, established from
  [primary Apple headers and jq disassembly](reverse-engineering/jq-libc-abi.md).
- Separate Darwin FILE objects and standard-stream pointer exports backed by
  host CRT streams. Binary I/O, EOF/error state, flush/close and slow paths.
- SysV printf-family register/stack capture and explicit va_list conversion;
  LP64 integer and floating formatting through typed host operations.
- UTF-8 path adapters, Darwin stat64 translation, selected open flags and
  `/C/...` drive paths recognized as absolute by unchanged POSIX consumers.
- Case-sensitive UTF-8 guest environment with empty/absent distinction;
  signed UTC calendar conversion, selected date formats and time adapters.
- jq's remaining memory/string/math imports, SysV sorting callbacks, random
  values, Darwin stack probes/checks and the 80-bit nexttoward argument bridge.
- Inherited stdin and explicit guest exit callbacks/stream flushing.

`build.rs` compiles the native bridge with Clang using warnings as errors.
Guest and Windows FILE/va_list/stat/tm layouts are never substituted for each
other. Unsupported locale names and formatting families fail explicitly.

## Verification

`python tools/verify.py` passes formatting, Clippy (`-D warnings`), Rust tests,
debug build, pinned artifact and test-data validation, Mach-O fixture
generation, archive tests, debug integration, release build and release
integration.

The current run passed **90 distinct tests:** 17 Rust, nine archive and 64
integration. All 64 integration tests also passed against the release build.
The upstream jq suite is one of those 64 methods and expands to 550 assertions.
Earlier passing summaries remain in
[summary-previous-pass.json](test-results/summary-previous-pass.json) and
[jq-execution-previous-pass.json](test-results/jq-execution-previous-pass.json).

An intermediate revision was blocked by Windows Application Control
(WinError/os error 4551), which also prevented Rust unit-test execution. The
current revision completed every verification stage, including both integration
profiles; no signing or security policy was modified. See
[the current run](test-results/latest.json) and
[current summary](test-results/summary.json). The historical blocked run is
retained in [release-policy-block-stdio.json](test-results/release-policy-block-stdio.json).

The 13 jq methods cover version/help, piped JSON transforms, a 6,000-record
stream, seeded sorting/grouping, Unicode/regex/escaping, CRLF/NUL preservation,
Unicode filenames/slurp/rawfile, filter files and module search, missing-file/
directory diagnostics, parse/compile/halt statuses, UTC dates/negative epoch,
math including nexttoward, environment and CLI variables. They assert expected
outputs and errors; they are not a full upstream jq suite. A reference-Mac run now
exists for a fixed CLI slice: the same 871,768-byte artifact produced byte-identical
stdout, stderr and exit status on a hosted Intel Mac and through Adapter for the
jq cases of the [CLI differential](differential-testing.md#recorded-cli-run-2026-10-22).

Three focused libc integration tests exercise classic/chained Mach-O consumers:
GP/SSE register exhaustion and stack arguments, 64-bit longs, va_list, width/
precision, truncation, count stores, rejected formats, extended-precision
direction, stream pointers/flags and raw bytes; explicit exit/callback flush;
UTF-8 file writes/reads, guarded stat64 buffers, error translation and UTC time.
Existing dylib/thread/parser/crash/timeout regressions remain passing.

Target: [jqlang jq-1.8.2](https://github.com/jqlang/jq/releases/tag/jq-1.8.2),
asset `jq-macos-amd64`, 871,768 bytes, SHA-256
`e94b266e3c26690550006abe63152b782280f4e14374accdf04cbde844f00bc0`.
Static preflight resolves 1,357 fixups in one image. Fetch/provenance is
reproducible through `tools/fetch_jq.py`; generated/downloaded files stay ignored.

## Upstream suite

The published jq 1.8.2 source archive is pinned by SHA-256
(`188d03d029e8242a366d2286f07638924afe997120f6057e606f3c7a33984312`,
1,440,150 bytes) and `tools/fetch_jq_tests.py` extracts only `tests/jq.test` and
`tests/modules`. Running the unchanged macOS binary with
`-L <modules> --run-tests <jq.test>` reports:

```
550 of 550 tests passed (0 malformed, 0 skipped)
```

Both debug and release workers pass; the run is part of
`test_upstream_test_suite`. No jq source is rebuilt and no reference-Mac output
is compared. The suite exercises module loading, error handling, regex,
Unicode, recursion and jq's own built-in C tests.

The first run exhausted the worker stack after test 491. Windows MSVC reserves
1 MiB for the main thread while Darwin reserves 8 MiB. `build.rs` now links
`/STACK:8388608`; the suite then completed. This is a host-environment fix, not
evidence that every deep-recursion workload is equivalent to macOS.

REA CLI 6.4.0 with a locally installed Ghidra 12.1.4 still rejects this Mach-O
target (`target_format_unsupported`: "Windows Ghidra P0 accepts PE targets
only"), so the suite result is runtime evidence from Adapter itself, cross-checked
with the pinned jq source and LLVM inspection, not REA decompilation.

## Remaining boundaries

C/POSIX locale only; no Unicode/xlocale contract. Full FILE internals, cookie
streams and locking APIs are absent. Positional, wide and long-double formatting
is unsupported; field width/precision has a 1 MiB bound. Host formatting/math
details and errno/fenv behavior require broader tests. stat permissions/ownership
and ctime are approximate; directory descriptors are unsupported. strptime/
strftime cover selected formats and local time uses Windows CRT rules/ranges.
Direct environ mutation is unverified. See [the API contract](compatibility.md).

Reference-Mac equivalence, C++ TLV lifecycle, synchronization, runtime dylib
loading, Objective-C, Foundation/AppKit/GUI and ARM64 translation remain open.
Xcode remains an inspected future target. The [roadmap](roadmap.md) gives the
next acceptance gates; historical dylib and bootstrap reports remain available.
