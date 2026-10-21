# Supplied Xcode 27 artifacts: measured findings

Inspection date: 2026-10-21, Windows x64. Source files were supplied by the user
in `E:\Xcode\req files`; originals were opened read-only. Package names are
user-provided identity claims; Apple signature/certificate trust was not
validated. No extracted Apple program was executed.

## Input identity and coverage

| File | Bytes | SHA-256 |
|---|---:|---|
| Xcode_27.xip | 2014229334 | `6a270c53a5a0c5e0ac78125342d44c3cfff2716ec390373cd5e30834d80a67c3` |
| Command_Line_Tools_27.dmg | 523927913 | `4d0b8af69d054224466a69f3fd566bddd504369115382879448088c9fb54192f` |
| Additional_Tools_for_Xcode_27.dmg | 64968192 | `6774cdb1a583ea5b1715d1f20a1fab2dad8b5f9ef5f04fe43e75f88b279749d8` |
| Icon_Composer_27.dmg | 19897344 | `7e7767324826ae18e38aa124aa2d2e6722838ae0c438382029b090ccf7b5b3d1` |

The XIP is XAR v1 with Metadata and Content members. Content is PBZX with XZ
blocks and odc CPIO. Inventory covered 166799 entries / 613 PBZX blocks. Metadata
declares 10237133510 uncompressed bytes; actual decoded CPIO including headers
was 10275147776 bytes. Content SHA-1 matched the XAR-declared checksum. This
checks archive consistency, not authenticity. Selected regular files were copied;
symlinks were recorded without creating them. Full inventory is local at
`target/artifacts/xcode27-inspection/inventory.jsonl`.

7-Zip 26.04 was downloaded from its [official distribution](https://www.7-zip.org/download.html) and unpacked locally
under `target/analysis-tools/`; no system installation was performed. It traversed
the DMG/HFS images and reported tail-data warnings. The CLT executable package
was decoded directly from its XAR TOC because 7-Zip's PackageInfo/Distribution
outputs aliased its Payload. Direct decoding verified Payload SHA-1
`18c014d5da388496472d972af54d243386a64849`, inventoried 5420 CPIO entries, and
selected 13 real command tools. The decoded PackageInfo identifies
`com.apple.pkg.CLTools_Executables`, version `27.0.0.0.1788430756`.

43 selected Mach-O images and 18 plist files were inspected; this is not a full
analysis of every bundled binary. Raw local reports are under
`target/artifacts/raw-inspection`; compact reports with all undefined symbols,
commands, sections and selected entry symbols are in
[apple-inputs/targets.json](apple-inputs/targets.json). Bundle fields are in
[apple-inputs/bundles.json](apple-inputs/bundles.json).

## REA observations and boundary

REA CLI 6.4.0 was actually used. No Ghidra/Hopper/IDA engine is configured, and
REA native Mach-O/plist operations report `unsupported_host`: native macOS
utilities require macOS. No decompiled pseudocode or runtime tracing was
obtained. The current Codex session has no REA MCP tools registered.

| Actual operation | Retained result |
|---|---|
| inspect-artifact Additional_Tools DMG | `ev_0438185d3c29a5eeac50dd460ca5f96dc69ff80eb43871c42280b9421b68eeb0`; root hash only, child traversal requires native macOS adapter |
| inspect-artifact Xcode XIP | `target_unavailable`, unsupported binary format |
| inspect-artifact selected partial Xcode.app tree | `ev_94d3c78707f483015e525b331543a5b017faf7be6b044051f4f4500f1d6bbd97`; portable directory inventory |
| inspect-artifact selected partial Pixie.app tree | `ev_126d80f912876197550626d878d01fbcf0429ebe94f4c019bd7bd78ca0b38bbd`; portable directory inventory |

REA inventory Evidence describes the selected tree at capture time, not the full
Xcode archive. Extraction continued afterwards, so its directory root hash is a
historical snapshot. Original member identities are retained separately in the
XIP summary. These portable inventories do not establish Mach-O call behavior.
The following binary facts come from adapter-inspect and independent LLVM inspection,
not from a REA decompiler.

## Findings ledger

| Source / location | Observed static evidence | Confidence / inference | Implementation consequence |
|---|---|---|---|
| Xcode.app/Contents/Info.plist | Xcode 27.0; CFBundleVersion 25183.107.5; build 27A266; minimum macOS 26.6 | High, parsed plist; declared OS requirement | Identify this artifact precisely; do not assume older Intel Xcode behavior |
| Contents/MacOS/Xcode, Mach header at offset 0 | Thin little-endian CPU 0x0100000c (arm64), no universal wrapper; LLVM independently agrees | High, observed bytes | Current Intel runtime must reject; an ABI thunk cannot translate ARM instructions |
| Xcode `_main` at VM 0x100000a78 | Calls getenv, dlopen, dlsym, error reporting and AppKit startup | High, static calls; actual path taken is unobserved | Requires runtime dynamic loading and GUI initialization beyond direct imports |
| Xcode VM 0x100000af4..0x100000b24 | Loads string `@rpath/Frameworks/libIDEApplicationLoader.dylib`, calls dlopen at 0x100000b00, resolves IDEApplicationLoad at 0x100000b10, indirect call at 0x100000b24 | High, disassembly plus referenced strings; successful loading not observed | Direct dependency list is incomplete; investigate this loader next |
| Xcode `_CreateClasses`, VM 0x1000009e0..0x100000a74 | References IDEApplication / IDEApplicationController and objc_allocateClassPair, class_addMethod, objc_registerClassPair | High, static reference/call evidence | Objective-C support needs dynamic class construction, not only objc_msgSend |
| libIDEApplicationLoader.dylib, `_IDEApplicationLoad` at VM 0xac8 | Thin arm64 MH_DYLIB; direct DVTFoundation, DVTUserInterfaceKit and IDEKit loads, plus AppKit/Foundation/libobjc/libSystem/CF; 48 imports | High, independently extracted member, CPIO header/name and source SHA-256 checked; no execution | Startup follows a private framework closure, not a single launcher API shim |
| Xcode imports / LC_LOAD_DYLIB | AppKit, Foundation, libobjc, libSystem, CoreFoundation; 32 undefined symbols | High for direct imports only | Existing five CF APIs are insufficient; private framework closure remains unresolved |
| Xcode clang, ar, ld, lipo, strings, strip, dsymutil, xcodebuild, DVTFoundation samples | Every inspected image is thin arm64 | High for sampled files; not a claim every file in the package is ARM-only | Keep developer toolchain work separate and retain Intel milestone scope |
| CLT 13 command-tool samples | All inspected images are thin arm64, including clang, ctags, llvm-nm, llvm-otool | High for selected files | These executables cannot run through native x64 mapping alone |
| Additional Tools: 15/16 inspected executables | Universal x86_64 + arm64; CarPlay Simulator is thin arm64 | High for inspected headers | Real Intel Apple artifacts exist for future regression targets |
| Clipboard Viewer Intel slice | 55 undefined symbols, five direct libraries, Objective-C sections | High static evidence; execution unverified | Candidate after ObjC/Foundation/AppKit prerequisites; no current compatibility claim |
| Pixie Intel slice | 134 imports, 12 direct libraries including private SkyLight and ImageKit | High static evidence | Lower priority than an app without private graphics dependencies |
| Icon Composer 27 main + two extensions | Universal x86_64 + arm64; main has 115 imports / 26 direct libraries, SwiftUI + Swift runtime + private CoreSVG; plist minimum macOS 26.6 | High static evidence; bundled framework closure only partly extracted | Intel slice alone does not make it executable; Swift/GUI/private framework work remains |

VM locations are preferred Mach-O virtual addresses before slide. The local
LLVM disassembly is `target/artifacts/xcode27-inspection/xcode-main-disassembly.txt`.
LLVM's Mach-O printer emitted some unresolved Objective-C reference comments;
those comments were not used to infer class identities. No macOS reference
execution, GUI launch, code-signature validation, or native decompilation is
claimed.

## Resulting direction

The supplied files substantially improve real-binary evidence. They do not
provide Windows implementations of AppKit/Foundation/libobjc, and the Xcode
launcher is outside the current x86-64 execution architecture. Continue the
Intel loader/dylib/runtime milestone using real Intel Additional Tools as
analysis targets, with ARM translation and the Xcode private-framework startup
closure recorded as separate future work. The identified libIDEApplicationLoader
has now been extracted and inspected; the next Xcode-specific analysis targets
are its DVTUserInterfaceKit/IDEKit dependency closure. That requires substantial
additional framework/Objective-C investigation before a runtime implementation.

Selected SDK headers are extracted with provenance under
`target/artifacts/xcode27-sdk/`. The pthread condition-variable layouts and the
CPIO hardlink-canary behavior that affects those extracts are documented in
[thread lifecycle evidence](thread-lifecycle-evidence.md).
