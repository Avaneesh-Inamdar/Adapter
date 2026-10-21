# Verified session result — 2026-10-21

This records the initial bootstrap session. For the completed Intel dylib and
thread runtime follow-up, see [the dylib iteration](dylib-tls-results.md), then
[the latest jq/libc iteration](jq-libc-results.md).

Built a working native Windows Rust repository from the originally empty
workspace, then inspected the user-supplied Xcode packages before further
compatibility expansion.

## Actual verification

`python tools/verify.py` completed all nine stages: Rust formatting, Clippy
with warnings treated as errors, Rust tests, debug build, LLVM Mach-O fixture
generation, archive unit tests, debug integration tests, release build, and
release integration tests. After adding selective random-access XIP extraction,
the archive tests were rerun and the successful result/log updated.

| Test group | Passed | Failed |
|---|---:|---:|
| Rust parser/fixup/Windows VM tests | 15 | 0 |
| Python archive tests | 8 | 0 |
| Runtime integration tests, debug | 32 | 0 |
| Same runtime integration tests, release | 32 | 0 |

55 distinct tests passed; the 32 integration cases were also exercised in
release. The mutation test performs 5000 deterministic header mutations within
one test. Every result above is from an actual local run, not an estimate.
Exact commands, exit codes and logs: [test-results/latest.json](test-results/latest.json).

The hello Mach-O fixture printed
`DarwinBridge: Mach-O code executed on Windows` and returned 0 after constructor,
argument, allocation and byte-copy checks. The CF fixture printed its success
message after UTF-8, UTF-16 length, buffer bounds and retain/release checks.
Chained variants also passed. A separate fixture returned 37 and the Windows
worker preserved that exit code. Timeout, crash, malformed-input, unsupported
architecture, missing-library and unknown-symbol paths were exercised.

Diagnosed and fixed observed issues: the final __LINKEDIT segment need not have
a page-multiple size; exact @loader_path rpaths must resolve; lazy bind state
defaults to pointer type; modern LLD emits S_INIT_FUNC_OFFSETS constructors;
SG_READ_ONLY segments must become read-only after fixups. Direct VirtualQuery
tests verify final protection and release. The early missing-library test also
had a length-changing mutation; it was corrected to preserve the binary layout.

## Real-binary evidence

User inputs were retained read-only. Inventoried all 166799 XIP entries and
the CLT executable package; inspected 43 selected Mach-O images / 18 plists.
Xcode 27.0 build 27A266, its sampled core tools, CLT samples and its dynamic
application loader are ARM64 only. Several Additional Tools and Icon Composer
have Intel slices. REA supplied portable inventory Evidence; native REA
Mach-O analysis was unavailable on this Windows host, so LLVM supplied the
header/disassembly cross-checks. No Apple executable was launched, no native
decompilation obtained, and no Apple signature trust established.

The public jq 1.8.2 macOS x64 asset was downloaded for static inspection and its
SHA-256 matched GitHub's release digest. It remains unsupported because of
Darwin TLS and missing runtime interfaces; it was not executed. Generated
Mach-O probes have not been compared with a reference Mac.

## Deliverables and next task

Build/use: [README](../README.md). Decisions: [architecture](architecture.md).
Evidence: [Xcode analysis](reverse-engineering/xcode27.md). Support:
[compatibility](compatibility.md). Gaps: [limitations](limitations.md).
Priorities: [roadmap](roadmap.md). Exact new repository files:
[file-changes](file-changes.md).

Phases 0-2 have their initial vertical slice, phase 3 has constrained native
execution, and phase 4 has a small verified CF string consumer. Objective-C,
Foundation, GUI, real application compatibility, and ARM execution remain
unimplemented/unverified. Next implementation: load the existing local Intel
dylib fixture, resolve its export, and establish dependency initialization
ordering before extending the real application corpus.
