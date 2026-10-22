# dyld unwind-section discovery

Recorded 2026-10-22. This P1 slice discovers mapped unwind metadata. **It does not
unwind frames, execute DWARF CFI or propagate C++/Objective-C exceptions.**

## Implemented

`_dyld_find_unwind_sections` locates the guest image containing an address and
returns its Mach header plus the address/length of `__TEXT,__eh_frame` and
`__TEXT,__unwind_info`. Missing sections are null/zero; a recognized image still
returns true. Unknown addresses return false without changing the output.
`dyld_image_header_containing_address` and
`dyld_image_path_containing_address` use the same range records.

The result is five LP64 words (40 bytes, alignment 8), matching
[Apple's pinned dyld header](https://github.com/apple-oss-distributions/dyld/blob/fd8d0c4d52320ebf64db34f3cb280310d905c5ae/include/mach-o/dyld_priv.h).
The success/failure behavior is grounded in the same revision's
[DyldAPIs.cpp](https://github.com/apple-oss-distributions/dyld/blob/fd8d0c4d52320ebf64db34f3cb280310d905c5ae/dyld/DyldAPIs.cpp).
The `dyld-unwind` entry in [contracts.json](abi/contracts.json) specifies
ownership, lifecycle and exclusions. Implementation is in
[dyld.rs](../src/darwin/dyld.rs), with relocated records supplied by
[runtime/mod.rs](../src/runtime/mod.rs) and
[runtime/dynamic.rs](../src/runtime/dynamic.rs).

Image containment covers actual mapped segments, including data, and excludes
PAGEZERO and allocation holes. Sections must be unique readable file-backed
TEXT ranges with matching VM/file offsets. Invalid metadata fails before guest
constructors. Plugin records are published before their constructors, and the
existing loader keeps those mappings after `dlclose`.

## Verification

[unwind_lookup.c](../tests/fixtures/unwind_lookup.c) exercises function/data/header
addresses, unknown and stack addresses, unchanged failure output, buffer guards,
section lengths/content prefixes and path/header identity. Local fixtures use
classic/chained fixups and three forms: normal compact tables, a DWARF fallback
and no unwind sections. A plugin's constructor queries its own metadata, proving
the dynamic registration is available before initialization.

The DWARF form uses a CFI expression for the existing frame-pointer CFA in one
function. That forces a real FDE fallback using a directive accepted by both
recorded compilers. Other functions may still have compact entries; the test
does not assume the compact index disappears. It does not throw an exception.

[Five integration methods](../tests/integration/test_unwind.py) cover six startup
variants, four plugin variants and rejection of inconsistent offsets, escaping
VM ranges and duplicate unwind sections. The full verifier passes **192 distinct
tests: 32 Rust, 30 Python unit/archive and 130 integration**. All 130 integration
tests also pass in release, including the ARC regressions.

## Mac reference

[Run 39813333933](https://github.com/Avaneesh-Inamdar/Adapter/actions/runs/39813333933),
workflow commit `3e5c83b`, built and executed the compact and DWARF-fallback
consumers on standard `macos-15-intel`, Darwin 24.6.0 x86_64, Apple clang 17.0.0.
The downloaded executables run unchanged through Adapter: **1,024/1,024 records
match for each suite in both debug and release**.

The suites are `adapter-unwind-compact-v1` and `adapter-unwind-dwarf-v1`.
Each repeats eight lookup families against one executable. Absolute addresses
and platform-specific path spelling are reduced to within-process identities;
section sizes and content prefixes compare exactly from the identical artifact.
These records do not count as stack-walking or exception tests. Native plugin
unload behavior is not covered by this reference run.

## Remaining work

Decode compact indexes and encodings; parse DWARF CIE/FDE augmentation and
encodings; evaluate CFI and recover registers; implement personalities and
exception propagation across guest/host frames. Add reference throws, cleanup,
cross-image unwinds and failure paths. Shared-cache/JIT unwind registration,
true unload and Windows SEH interoperability remain open. The current host
LLDB debugger bridge is a separate capability.

## Retained evidence

| Suite | Executable SHA-256 | Source SHA-256 |
| --- | --- | --- |
| `compact` | `0b376c9e96d0d14b686607bef187f55d8fbe2908297752cb751cfcdb6d4cbdae` | `5d50264a5e2e2f955fcdc531dd78a8f700fc165d431123ff48d5b58fc1405dd2` |
| `dwarf` | `1a9e5a1f43643e1ee001a4616fb8a1506a576740b40c79305d5dc0ffa479a3a6` | `5d50264a5e2e2f955fcdc531dd78a8f700fc165d431123ff48d5b58fc1405dd2` |

| Capture/report | SHA-256 of stored bytes |
| --- | --- |
| [test-results/mac-reference-unwind-compact-39813333933.json.gz](test-results/mac-reference-unwind-compact-39813333933.json.gz) | `14da25ffc812cb53e2c656c347660059820fc8de7493a34e306d58d706acffc4` |
| [test-results/differential-unwind-compact-candidate-debug.json.gz](test-results/differential-unwind-compact-candidate-debug.json.gz) | `9268da034a119adad81117a36fb9fc3f6e3d6bb2321f9a21ab761eb5f79af204` |
| [test-results/differential-unwind-compact-debug.json](test-results/differential-unwind-compact-debug.json) | `f012c3d391a1cc44430cf0bdb0859f4e420e3d49a7e3884af8b85c068813c4c8` |
| [test-results/differential-unwind-compact-candidate-release.json.gz](test-results/differential-unwind-compact-candidate-release.json.gz) | `bc56af5c24c3663e2cb34b7f69cf503732abd9b1187cfea48bd732a6e027793b` |
| [test-results/differential-unwind-compact-release.json](test-results/differential-unwind-compact-release.json) | `96d98f1e4bbcb93f7852870925a5c294321a1ebba194de2c7b4302892348ecfc` |
| [test-results/mac-reference-unwind-dwarf-39813333933.json.gz](test-results/mac-reference-unwind-dwarf-39813333933.json.gz) | `ae45744883f57efca3d0c2ae68f175c619cb43b3d96ddd3cc0bc10bb80829996` |
| [test-results/differential-unwind-dwarf-candidate-debug.json.gz](test-results/differential-unwind-dwarf-candidate-debug.json.gz) | `de9d3c3d12e0816c5abb88f34bb8e5baeba1c995ecf112eac5ecdabed1de4729` |
| [test-results/differential-unwind-dwarf-debug.json](test-results/differential-unwind-dwarf-debug.json) | `5e13835816f7c510bf532e0c6ad5f0dd648e4014de7c548723fdba55540229e8` |
| [test-results/differential-unwind-dwarf-candidate-release.json.gz](test-results/differential-unwind-dwarf-candidate-release.json.gz) | `8a60220ebb53d9656970773631240c208f6d108894cf1712607192b4899dfb86` |
| [test-results/differential-unwind-dwarf-release.json](test-results/differential-unwind-dwarf-release.json) | `f194cfaa3bdd724dcc01f2eb01db40055197778be266311c78154e32760a3302` |

The compressed captures retain all raw output, parsed records, exit/host identities and build provenance. Decode before using `tools/differential.py compare`. The workflow artifact additionally retains exact executables and manifests for 14 days; the captures remain in Git.
