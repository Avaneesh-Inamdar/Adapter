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

An earlier `python tools/verify.py` run passed formatting, Clippy (`-D warnings`), Rust tests,
debug build, pinned artifact validation, Mach-O fixture generation, archive
tests, debug integration, release build and release integration. Its passing
summary and sample outputs are retained in
[summary-previous-pass.json](test-results/summary-previous-pass.json) and
[jq-execution-previous-pass.json](test-results/jq-execution-previous-pass.json).

**89 distinct tests:** 17 Rust, nine archive and 63 integration. All 63
integration tests passed again against the release build in that earlier run.

**The final verification run is blocked by Windows Application Control.** After
final locale selection/FILE argument validation and native formatting changes,
the host denied executable launches with WinError/os error 4551. This also
prevented Rust unit-test execution. The verifier records blocked stages as
non-passing, continues independent checks, and reports per-profile evidence.
No signing/security policy was modified. See
[the final run](test-results/latest.json), [current summary](test-results/summary.json)
and [the initial release block](test-results/release-policy-block-stdio.json).
The added empty-fopen-mode assertion has compiled but could not execute in the
final run. This host limitation is not reported as a compatibility pass.

The 13 jq methods cover version/help, piped JSON transforms, a 6,000-record
stream, seeded sorting/grouping, Unicode/regex/escaping, CRLF/NUL preservation,
Unicode filenames/slurp/rawfile, filter files and module search, missing-file/
directory diagnostics, parse/compile/halt statuses, UTC dates/negative epoch,
math including nexttoward, environment and CLI variables. They assert expected
outputs and errors; they are not a full upstream jq suite or reference-Mac run.

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
