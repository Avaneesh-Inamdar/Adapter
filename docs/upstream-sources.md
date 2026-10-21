# Upstream sources and external dependencies

Research checked on 2026-10-22. This catalog covers the requested roadmap;
implementation status and acceptance gates belong in [the roadmap](roadmap.md)
and [README](../README.md). A source link does not count as an implemented or
tested Adapter feature.

The pinned revisions below are research snapshots, not a claim that they match
the private frameworks in the supplied Xcode 27 image. Before using a definition
in an ABI contract, record the particular header, revision, architecture and
matching binary evidence. No upstream code was imported by this catalog.

## Reference execution and development tools

| Requested item | Available code or service | What still has to happen |
| --- | --- | --- |
| 1. Real Mac or Mac cloud for ground truth | [GitHub-hosted macOS runners][runners] provide Intel and Apple silicon macOS execution. An Intel macOS job can capture native results for the same Intel Mach-O artifact subsequently run through Adapter. | Run and retain a successful reference job, exact artifact hashes, OS/build and architecture, stdout/stderr and exit status. Hosted execution is a macOS VM on Apple infrastructure; it does not establish physical-device, interactive desktop or GPU behavior. Reading source on Windows supplies no macOS observations. |
| 2. Thousands of differential probes | [LLVM's test infrastructure][llvm] and the existing Adapter fixture builders are useful starting points; a project-specific same-artifact capture/compare harness is still necessary. | Start with deterministic ABI probes and grow the corpus. Compare raw output and return codes; normalize only explicitly declared nondeterminism. Keep missing, failed and unsupported reference captures separate from passes. A Windows-only regression suite is not differential evidence. |
| 3. ARM64 execution decision | [Windows on Arm][woa] supports native ARM64 applications. [QEMU][qemu] has an AArch64 translator; [Unicorn][unicorn] exposes CPU emulation as a library. | Prefer a native Windows ARM64 port as the first proposed path, subject to available hardware and ABI probes. ARM64-to-x64 translation on the current x64 host is a separate implementation track. Neither project provides Adapter's Mach-O loader, Darwin personality or Apple framework ABI. No ARM runner result is implied by source research. |
| 4. Guest crash debugging | [LLDB][llvm] contains Mach-O/DWARF support and Darwin debugserver sources. [Windows debugging APIs][windebug] expose debug events, process/thread contexts and breakpoints. | Build the bridge: image/load-address records, faulting guest PC, symbolization, thread registers, breakpoints and single-step, plus reproducible crash artifacts. A native Windows debugger attaching to the worker is useful before a Darwin-compatible debugserver exists. |
| 5. Strict subsystem ABI contracts | [Apple's OSS distribution][apple-oss], [Clang ABI code][llvm] and pinned SDK/binary observations provide definitions. | Each subsystem needs its own versioned contract for layouts, calling convention, ownership, callbacks, errors, lifecycle, unsupported cases and reference probes. A shared C function name does not establish ABI equivalence. |
| 6. Binary pass/fail milestones | [AppKit documentation][appkit], [Xcode command-line documentation][xcode-tools] and the pinned supplied binaries define observable workflows. | Record separate results for NSWindow hello-world, xcodebuild, Xcode welcome window, opening a project and building that project. A lower-level API test cannot mark any of these passed. |

GitHub's runner documentation is a live service reference rather than a pinned
source revision. Check the available labels when enabling CI. The repository's
manual Mac workflow is the concrete route for collecting reference observations;
only its completed result establishes that a Mac comparison occurred. It does
not require a locally attached Mac or purchase of cloud hardware.

## Loading and language runtimes

| Requested item | Primary source | Reuse boundary and remaining work |
| --- | --- | --- |
| 7. macOS system image and dyld shared cache | [Apple dyld][dyld], including `other-tools/dsc_extractor.cpp`, cache format headers and loader implementation. | The extractor is reference implementation code, not a Windows-ready system image loader. Add input provenance, subcache discovery, slide/fixup handling, image identity and dependency loading, then cache-specific ObjC/Swift/TLV tests. Extracting an image does not make its APIs executable. User-supplied macOS system contents remain external inputs. |
| 8. Apple-ABI Objective-C runtime | [Apple objc4][objc4] contains class realization, selector registration, ownership, exception handling and architecture-specific messengers. [GNUstep libobjc2][libobjc2] has Windows support. | objc4 is the direct Apple ABI reference. libobjc2 documents GCC/GNUstep ABIs, so it cannot simply replace Apple's runtime for unchanged Mach-O consumers. Adapter must implement or bridge class layouts, registration, dispatch, super sends, ownership, initialization, categories, protocols and image lifecycle, with declared exclusions. |
| 9. Swift runtime | [Swift compiler and runtime][swift], ABI documents and runtime tests; [Swift's Windows distribution][swift-windows]. | A Windows Swift toolchain does not supply Apple's Mach-O runtime entry points and metadata interoperability. Pin a guest Swift ABI/version, then test metadata, reference counting, reflection, ObjC interaction, concurrency and exceptions. Native Windows Swift support is a porting resource. |
| 10. Darwin compact unwind and DWARF | [LLVM libunwind][unwind], especially `include/mach-o/compact_unwind_encoding.h`, plus libc++abi and their tests in [LLVM][llvm]. | Port lookup of loaded Mach-O unwind sections and exception personalities. Distinguish guest unwinding from Windows SEH; test cross-image C++, Objective-C and mixed guest/host frames. Merely parsing unwind sections provides no exception support. |
| 11. Mach VM | [XNU][xnu]: `osfmk/mach/mach_vm.defs`, `osfmk/vm/vm_map.c` and VM tests. | XNU implements these as kernel services. Define a guest address-space model over Windows mappings, protection limits, allocation flags, region queries, task rights, shared mappings and errors; avoid exposing host process handles as Mach ports. |
| 12. Page size and memory ordering | [Apple architecture differences][apple-arch], XNU's VM definitions and [Windows memory management][winpages]. [Apple's Rosetta TSO documentation][tso] explains its Apple-specific TSO enablement. | Preserve guest 16 KiB alignment/protection/query semantics where required while Windows uses its host granularity. The Apple TSO toggle described by Apple serves Rosetta's translated x86_64 code; it is not a blanket requirement for native ARM64 macOS apps. Do not assume that Windows ARM hardware exposes Apple TSO controls. Use architecture-specific atomic and ordering probes. |
| 13. arm64e pointer authentication | [Clang pointer-authentication specification][ptrauth], [Apple PAC documentation][apple-pac], objc4 and dyld. | Authentication keys, discriminators, address diversity, signed return addresses and authenticated chained fixups require a deliberate execution model. Stripping high bits is not equivalent to preserving authentication semantics. Plain arm64 support does not prove arm64e support. |

## System services

| Requested item | Primary source | Reuse boundary and remaining work |
| --- | --- | --- |
| 14. libdispatch / GCD | [Apple libdispatch][dispatch] and [Swift corelibs libdispatch][swift-dispatch], including `src/event/event_windows.c`. | A Windows event backend already exists upstream. Assess it before writing a new scheduler, but keep Apple's guest calling convention, Blocks ABI, queue lifecycle, QoS and Mach dispatch-source contracts explicit. Successful native Windows dispatch tests do not establish Darwin ABI compatibility. |
| 15. Full CoreFoundation and Foundation | [Apple CFLite][cf], [Swift corelibs Foundation][corelibs-foundation], [Swift Foundation][swift-foundation], [GNUstep Base][gnustep-base]. | CFLite's README explicitly says it is not the exact shipping macOS/iOS implementation. Swift corelibs provides Foundation for non-Darwin platforms; GNUstep implements a Foundation-compatible API under its runtime. All require an Apple object/CF toll-free-bridging and binary ABI assessment. Grow from the current string subset with collections, resources, bundles, property lists and object lifetimes. |
| 16. CFRunLoop on Mach ports | [Apple `CFRunLoop.c`][cf-runloop] and [corelibs CoreFoundation][corelibs-foundation]. | Preserve sources, observers, timers, modes, wakeups and Mach-port callbacks against the broker's port-right model. A Windows event loop alone does not establish these semantics. |
| 17. Complex Mach messages | [XNU message definitions][mach-message] and IPC implementation under `osfmk/ipc`. | Add port and OOL memory descriptors, right dispositions, copy/deallocate rules, descriptor validation, cleanup after partial failure and cross-process tests. The existing simple-message broker is only a starting point. |
| 18. XPC | [Apple XPC API documentation][xpc] and [Darling's implementation tree][darling] provide public API and independent implementation evidence. | No complete current Apple libxpc source was established by this research. Extend Adapter's typed-message subset only through a written transport, Blocks, ownership, cancellation, endpoint, entitlement and error contract. A broker round trip does not demonstrate interoperability with Apple's wire protocol. |
| 19. Real launchd semantics | [Apple launchd][launchd] contains job management and bootstrap code. | The published snapshot is older than the supplied macOS/Xcode target. Preserve the existing subset, then add domains, job lifecycle, KeepAlive, socket activation, launchctl, inherited rights and restart rules against current Mac traces. Historical source is evidence, not a full specification of modern launchd. |
| 20. fork / exec / posix_spawn | [XNU][xnu]: `bsd/kern/kern_fork.c`, `kern_exec.c`; [Libc][libc]: `sys/posix_spawn.c`. | Start with spawn and explicit descriptor/argument/environment inheritance. Windows process creation does not reproduce fork's address-space snapshot, pthread-atfork or signal semantics. Keep fork as its own acceptance gate. |
| 21. Signals and syscall personality | [XNU][xnu]: `bsd/kern/kern_sig.c`, syscall tables, trap handling and public signal layouts; [Libc][libc]. | Define guest numbering, saved contexts, delivery/masking, alternate stacks, restart behavior and direct trap interception. Existing CRT wrappers cannot cover direct Darwin syscalls. |
| 22. IOKit / sysctl / proc_info | [IOKitUser][iokit]; [XNU][xnu]: `bsd/sys/sysctl.h`, `bsd/sys/proc_info.h` and kernel implementations. | IOKitUser talks to a Darwin kernel registry and user clients. Supply target-specific registry/device/process facts and stable guest identifiers; define unsupported selectors explicitly. Do not substitute Windows structure layouts. |
| 23. Daemons | [CF preferences][cf-preferences], [Libnotify / notifyd][notify], [Security / securityd / trustd][security], [OpenDirectory][opendirectory] and [DirectoryService][directoryservice]. [LaunchServices documentation][launchservices] defines the public surface. | Available code covers portions of this stack. This research did not establish complete current cfprefsd, distnoted, LaunchServices or opendirectoryd server implementations. Inventory each daemon's IPC calls, persistence and session/identity requirements separately; port or implement a measured service. Security contains daemon code, but relies on Apple platform services. |
| 24. FSEvents / Spotlight | [Apple FSEvents][fsevents] and [Core Spotlight][spotlight] documentation; XNU's `bsd/vfs/vfs_fsevents.c` is lower-level event evidence. | Provide a durable event stream, rescan/drop behavior, volume/path identities and an indexing/search contract. Windows directory notifications do not automatically supply persistent FSEvents IDs or Spotlight query semantics. No full Apple Spotlight indexer implementation was found in the reviewed sources. |

## Graphics and UI

| Requested item | Primary source | Reuse boundary and remaining work |
| --- | --- | --- |
| 25. CoreGraphics / CoreText / CoreAnimation / ImageIO / ColorSync | [Microsoft WinObjC][winobjc] has Windows implementations of portions of these iOS APIs. [Darling][darling] has independent framework implementations. Apple documents [CoreGraphics][cg], [CoreText][ct], [QuartzCore][ca], [ImageIO][imageio] and [ColorSync][colorsync]. | WinObjC is archived and builds Windows apps from source. It is a selective backend/reference candidate, not a binary loader for Apple frameworks. Audit each API, geometry/text layout, object ownership, font metrics, image formats, color profiles and compositor behavior against a chosen consumer. No complete current Apple implementations were established here. |
| 26. IOSurface | [Apple IOSurface API][iosurface] and [Darling's IOSurface tree][darling]. | Define allocation, pixel layout, process sharing, lookup/rights, synchronization and D3D resource interoperability. An opaque buffer with the same API names does not provide cross-process IOSurface behavior. |
| 27. Metal to D3D12, AIR to DXIL | [Microsoft DXC][dxc] compiles HLSL into DXIL/SPIR-V. [Metal documentation][metal] specifies the Apple API. | Add Metal object/resource/synchronization and shader translation contracts separately. DXC does not decode Apple AIR. DXMT translates D3D11/10 to Metal, the opposite direction. No verified production-ready AIR-to-DXIL implementation was identified in this research. Compiled shader compatibility remains a research phase with pinned AIR samples and GPU comparisons. |
| 28. AppKit | [GNUstep GUI][gnustep-gui], [Darling][darling] and [Apple AppKit API][appkit]. | GNUstep supplies useful behavior/backend code under its own runtime and licenses. Native Apple class metadata, nib/storyboard resources, responder chains, event dispatch, accessibility and private AppKit interfaces still need an Adapter contract. First milestone is a real NSWindow consumer, followed by lifecycle and input tests. |
| 29. SwiftUI | [Apple SwiftUI API documentation][swiftui] and [Swift runtime][swift]. | Apple's shipping SwiftUI implementation is not supplied by the Swift compiler repository. Plan it after Swift, ObjC, Foundation, rendering and window/event support. A declarative UI recreation or open implementation does not establish compatibility with the supplied private SwiftUI binary. |
| 30. WindowServer replacement | [Win32 window APIs][winwindows], [UI Automation][uia] and the Windows graphics APIs are host building blocks. | Implement and test window lifetime, coordinates/scaling, input/IME, focus, menus, pasteboard, drag/drop and accessibility. Map the calls needed by AppKit/CoreGraphics; no current complete Apple WindowServer source was established here. |

## Xcode-specific services

| Requested item | Primary source | Reuse boundary and remaining work |
| --- | --- | --- |
| 31. LLDB / debugserver Mach primitives | [LLVM debugserver MachTask][mach-task], [MachException][mach-exception], plus XNU task/thread/exception definitions. | Implement task_for_pid rights/policy, thread state flavors, suspend/resume, memory access and exception-port delivery on top of Windows debug events. LLDB's portable host support does not supply these guest Mach services. |
| 32. codesign / Security / keychain | [Apple Security][security] includes code-signing, keychain, securityd and trustd sources with substantial Darwin dependencies. | Separate signature parsing/verification, guest policy/entitlements, certificate trust and persistent credentials. Define a test keychain store, signing inputs and trust fixtures explicitly; a Windows certificate check is not Apple's full policy. |
| 33. xcrun / xcode-select / SDK paths | [Apple command-line tools documentation][xcode-tools], supplied Xcode artifacts and their hashes. | Add selected developer-directory state, tool and SDK discovery, platform bundles, version/build queries and subprocess execution. Some tools ship only with full Xcode. Do not replace xcodebuild's pass gate with a host clang invocation or SDK file extraction. |
| 34. Instruments / DTX | [pymobiledevice3 DVT documentation][dvt] describes its independent DTX implementation; [Apple DTrace][dtrace] supplies a different, lower-level tracing component. | These are protocol/tracing references, not the full Instruments application or all its services. Pin a protocol implementation before reuse, record its license and device/OS support, then capture actual service conversations. No current full Apple DTX implementation was established by this research. |
| 35. Physical devices / USB / usbmuxd / CoreDevice / pairing | [libimobiledevice][libimobiledevice], [usbmuxd][usbmuxd] and [pymobiledevice3][pymobiledevice] are independent protocol implementations. | Test Windows transport/drivers, pairing/trust records, developer mode, device-service versions and modern CoreDevice/RemoteXPC flows separately. Existing mobile-device protocol support does not prove Xcode CoreDevice compatibility. Actual physical-device acceptance needs a device; source cannot produce that observation. |
| 36. CoreSimulator and iOS runtime | [Apple simulator documentation][simulator] describes the Xcode-supplied device/runtimes; [Meta idb][idb] controls Apple's installed CoreSimulator stack. | Defer until macOS guest execution and Xcode's earlier milestones work. idb is an automation client, not a replacement for CoreSimulator or iOS frameworks. No complete Apple CoreSimulator/iOS runtime source was established here, and installing an automation tool does not implement those services. |

## Pins and licenses

The revisions below are immutable links. License descriptions identify the
reviewed repository's principal license; individual files and bundled third-party
code can differ. Preserve their notices and check the specific files before
copying or linking them. Research citations do not add these projects as Adapter
dependencies or change Adapter's own license.

| Project | Research revision | License reference |
| --- | --- | --- |
| [apple-oss-distributions/dyld][dyld] | `fd8d0c4d5232` | [APSL-2.0; check file headers](https://github.com/apple-oss-distributions/dyld/blob/fd8d0c4d52320ebf64db34f3cb280310d905c5ae/APPLE_LICENSE) |
| [apple-oss-distributions/objc4][objc4] | `fb2650982983` | [APSL-2.0; check file headers](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/APPLE_LICENSE) |
| [apple-oss-distributions/xnu][xnu] | `f6217f891ac0` | [APSL-2.0 and other per-file licenses](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/APPLE_LICENSE) |
| [apple-oss-distributions/Libc][libc] | `71bbe350ab79` | [APSL-2.0 / BSD and other per-file licenses](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/APPLE_LICENSE) |
| [apple-oss-distributions/launchd][launchd] | `d448a1c8f70a` | [Apache-2.0 in reviewed src/launchd.c; inspect each file](https://github.com/apple-oss-distributions/launchd/blob/d448a1c8f70a61202f8705f94337f686b87c30c4/src/launchd.c) |
| [apple-oss-distributions/libdispatch][dispatch] | `2361ffb78a76` | [Apache-2.0; check file headers](https://github.com/apple-oss-distributions/libdispatch/blob/2361ffb78a76f7ee488cd052eb0bc5c767118bf9/LICENSE) |
| [apple-oss-distributions/CF][cf] | `dc54c6bb1c1e` | [APSL-2.0; check file headers](https://github.com/apple-oss-distributions/CF/blob/dc54c6bb1c1e5e0b9486c1d26dd5bef110b20bf3/APPLE_LICENSE) |
| [apple-oss-distributions/IOKitUser][iokit] | `323ead896d04` | [APSL-2.0; check file headers](https://github.com/apple-oss-distributions/IOKitUser/blob/323ead896d04424f87184d8f6ff0cce811aab106/APPLE_LICENSE) |
| [apple-oss-distributions/Libnotify][notify] | `715d461778f6` | [APSL-1.1 top level; inspect per-file notices](https://github.com/apple-oss-distributions/Libnotify/blob/715d461778f6b93c821d99390a0078bd6f6d8c04/APPLE_LICENSE) |
| [apple-oss-distributions/Security][security] | `db15acbe6a7f` | [APSL-2.0 and substantial third-party/per-file licenses](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/APPLE_LICENSE) |
| [apple-oss-distributions/OpenDirectory][opendirectory] | `ced844f36c8a` | [APSL-2.0; check file headers](https://github.com/apple-oss-distributions/OpenDirectory/blob/ced844f36c8a1c5b548cf4047f5620c34fa507da/APPLE_LICENSE) |
| [apple-oss-distributions/DirectoryService][directoryservice] | `e26acc499a53` | [APSL-2.0; check file headers](https://github.com/apple-oss-distributions/DirectoryService/blob/e26acc499a5326e46fb544ba56ad05e778c96b28/APPLE_LICENSE) |
| [apple-oss-distributions/dtrace][dtrace] | `4e57fe5c4dcf` | [APSL-2.0 and per-file licenses](https://github.com/apple-oss-distributions/dtrace/blob/4e57fe5c4dcf71e455817e7f26fc1f68e6eb4fe2/APPLE_LICENSE) |
| [swiftlang/swift][swift] | `cf88b986add1` | [Apache-2.0 with runtime library exception; bundled code varies](https://github.com/swiftlang/swift/blob/cf88b986add15769cef36f088647d91c4318df07/LICENSE.txt) |
| [swiftlang/swift-corelibs-foundation][corelibs-foundation] | `b40c41e73612` | [Apache-2.0 with runtime library exception; bundled code varies](https://github.com/swiftlang/swift-corelibs-foundation/blob/b40c41e73612385c24be821bdfe95a67fdb157a5/LICENSE) |
| [swiftlang/swift-foundation][swift-foundation] | `c8cc07611347` | [Apache-2.0 with runtime library exception; bundled code varies](https://github.com/swiftlang/swift-foundation/blob/c8cc07611347b283e0acc8a0da03738a42db9ef0/LICENSE.md) |
| [swiftlang/swift-corelibs-libdispatch][swift-dispatch] | `ce1b4d354042` | [Apache-2.0 with runtime exception](https://github.com/swiftlang/swift-corelibs-libdispatch/blob/ce1b4d3540427a8c8a17a3b5eb7c5749665d384e/LICENSE) |
| [llvm/llvm-project][llvm] | `3878e6804454` | [Apache-2.0 with LLVM exception; third-party code varies](https://github.com/llvm/llvm-project/blob/3878e680445418b076d7dd860f15171f22747751/LICENSE.TXT) |
| [darlinghq/darling][darling] | `60ba801decee` | [GPL-3.0 top level; submodules/files have separate licenses](https://github.com/darlinghq/darling/blob/60ba801decee7a00782f74f6be4c8ffb013f79ff/LICENSE) |
| [gnustep/libobjc2][libobjc2] | `a21ae931e4c1` | [MIT-style COPYING; bundled/per-file code varies](https://github.com/gnustep/libobjc2/blob/a21ae931e4c186312701d7a4a8e85204955965a6/COPYING) |
| [gnustep/libs-base][gnustep-base] | `85dedc64bd65` | [LGPL; tools and individual files can differ](https://github.com/gnustep/libs-base/blob/85dedc64bd65492a5c0f0f80516688e2db59d57f/COPYING.LIB) |
| [gnustep/libs-gui][gnustep-gui] | `cacd5cb08971` | [LGPL; tools and individual files can differ](https://github.com/gnustep/libs-gui/blob/cacd5cb0897121c0003f625ab2db5b6152556ce9/COPYING.LIB) |
| [qemu/qemu][qemu] | `8967b2fe1b2a` | [GPL-2.0 top level; individual components vary](https://github.com/qemu/qemu/blob/8967b2fe1b2aaae3c29e207d85fab901d580a2dc/COPYING) |
| [unicorn-engine/unicorn][unicorn] | `7c5db94191de` | [GPL-2.0 top level; components vary](https://github.com/unicorn-engine/unicorn/blob/7c5db94191defc1e04a4f66f4eb1220903cba837/COPYING) |
| [libimobiledevice/libimobiledevice][libimobiledevice] | `fa0f79190142` | [LGPL-2.1 library; tools/components vary](https://github.com/libimobiledevice/libimobiledevice/blob/fa0f79190142bc309307967c058f89c1b36eb6b8/COPYING.LESSER) |
| [libimobiledevice/usbmuxd][usbmuxd] | `3ded00c9985a` | [GPL-2.0/GPL-3.0; check source notices](https://github.com/libimobiledevice/usbmuxd/blob/3ded00c9985a5108cfc7591a309f9a23d57a8cba/COPYING.GPLv3) |
| [microsoft/DirectXShaderCompiler][dxc] | `c3a85328eb12` | [LLVM/NCSA and other component licenses](https://github.com/microsoft/DirectXShaderCompiler/blob/c3a85328eb12dffdba41026d6ec940c22c5308e0/LICENSE.TXT) |
| [microsoft/WinObjC][winobjc] | `94f6b5bfdc09` | [MIT top level; ThirdPartyNotices.txt applies](https://github.com/microsoft/WinObjC/blob/94f6b5bfdc09d945e30607b56124b6f94a5b9d7b/LICENSE.md) |
| [doronz88/pymobiledevice3][pymobiledevice] | `f6010ff51bc96` | [GPL-3.0; component notices apply](https://github.com/doronz88/pymobiledevice3/blob/f6010ff51bc963884e688a2160db50b6b9a68b54/LICENSE) |
| [facebook/idb][idb] | `56cd11d06e90` | [MIT top level; component notices apply](https://github.com/facebook/idb/blob/56cd11d06e901b70650cbbe76bc12e41806a31e7/LICENSE) |

Apple OSS code often supplies the ABI definitions and Darwin implementation
needed for comparison. Kernel code cannot be linked into a Windows user process
to provide its kernel semantics. Public source releases can lag a current macOS
binary, and proprietary frameworks/private services need separate binary and
runtime evidence.

Darling targets Linux. Its Mach service, Linux kernel and framework integration
cannot be dropped into the Windows runtime. GNUstep and WinObjC target a source
development environment; matching their APIs does not establish Apple's class
metadata, calling convention or binary ABI.

No additional skill or MCP installation was needed for this research. The
concrete gaps are runtime implementations, reference observations and hardware
execution paths; adding an agent tool does not fill them.

[dyld]: https://github.com/apple-oss-distributions/dyld/tree/fd8d0c4d52320ebf64db34f3cb280310d905c5ae
[objc4]: https://github.com/apple-oss-distributions/objc4/tree/fb265098298302243cd7eeaa1f63f0ba7786dd9a
[xnu]: https://github.com/apple-oss-distributions/xnu/tree/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea
[libc]: https://github.com/apple-oss-distributions/Libc/tree/71bbe350ab79eef58113991d817ccc6165061a64
[launchd]: https://github.com/apple-oss-distributions/launchd/tree/d448a1c8f70a61202f8705f94337f686b87c30c4
[dispatch]: https://github.com/apple-oss-distributions/libdispatch/tree/2361ffb78a76f7ee488cd052eb0bc5c767118bf9
[cf]: https://github.com/apple-oss-distributions/CF/tree/dc54c6bb1c1e5e0b9486c1d26dd5bef110b20bf3
[iokit]: https://github.com/apple-oss-distributions/IOKitUser/tree/323ead896d04424f87184d8f6ff0cce811aab106
[notify]: https://github.com/apple-oss-distributions/Libnotify/tree/715d461778f6b93c821d99390a0078bd6f6d8c04
[security]: https://github.com/apple-oss-distributions/Security/tree/db15acbe6a7f257a859ad9a3bb86097bfe0679d9
[opendirectory]: https://github.com/apple-oss-distributions/OpenDirectory/tree/ced844f36c8a1c5b548cf4047f5620c34fa507da
[directoryservice]: https://github.com/apple-oss-distributions/DirectoryService/tree/e26acc499a5326e46fb544ba56ad05e778c96b28
[dtrace]: https://github.com/apple-oss-distributions/dtrace/tree/4e57fe5c4dcf71e455817e7f26fc1f68e6eb4fe2
[swift]: https://github.com/swiftlang/swift/tree/cf88b986add15769cef36f088647d91c4318df07
[corelibs-foundation]: https://github.com/swiftlang/swift-corelibs-foundation/tree/b40c41e73612385c24be821bdfe95a67fdb157a5
[swift-foundation]: https://github.com/swiftlang/swift-foundation/tree/c8cc07611347b283e0acc8a0da03738a42db9ef0
[swift-dispatch]: https://github.com/swiftlang/swift-corelibs-libdispatch/tree/ce1b4d3540427a8c8a17a3b5eb7c5749665d384e
[llvm]: https://github.com/llvm/llvm-project/tree/3878e680445418b076d7dd860f15171f22747751
[darling]: https://github.com/darlinghq/darling/tree/60ba801decee7a00782f74f6be4c8ffb013f79ff
[libobjc2]: https://github.com/gnustep/libobjc2/tree/a21ae931e4c186312701d7a4a8e85204955965a6
[gnustep-base]: https://github.com/gnustep/libs-base/tree/85dedc64bd65492a5c0f0f80516688e2db59d57f
[gnustep-gui]: https://github.com/gnustep/libs-gui/tree/cacd5cb0897121c0003f625ab2db5b6152556ce9
[qemu]: https://github.com/qemu/qemu/tree/8967b2fe1b2aaae3c29e207d85fab901d580a2dc
[unicorn]: https://github.com/unicorn-engine/unicorn/tree/7c5db94191defc1e04a4f66f4eb1220903cba837
[libimobiledevice]: https://github.com/libimobiledevice/libimobiledevice/tree/fa0f79190142bc309307967c058f89c1b36eb6b8
[usbmuxd]: https://github.com/libimobiledevice/usbmuxd/tree/3ded00c9985a5108cfc7591a309f9a23d57a8cba
[dxc]: https://github.com/microsoft/DirectXShaderCompiler/tree/c3a85328eb12dffdba41026d6ec940c22c5308e0
[winobjc]: https://github.com/microsoft/WinObjC/tree/94f6b5bfdc09d945e30607b56124b6f94a5b9d7b
[runners]: https://docs.github.com/en/actions/reference/runners/github-hosted-runners
[woa]: https://learn.microsoft.com/windows/arm/
[winpages]: https://learn.microsoft.com/en-us/windows/win32/winprog64/memory-management
[windebug]: https://learn.microsoft.com/en-us/windows/win32/debug/about-basic-debugging
[apple-oss]: https://github.com/apple-oss-distributions
[xcode-tools]: https://developer.apple.com/documentation/xcode/installing-the-command-line-tools/
[swift-windows]: https://www.swift.org/install/windows/
[apple-arch]: https://developer.apple.com/documentation/apple-silicon/addressing-architectural-differences-in-your-macos-code
[tso]: https://developer.apple.com/documentation/virtualization/accelerating-the-performance-of-rosetta
[apple-pac]: https://developer.apple.com/documentation/security/preparing-your-app-to-work-with-pointer-authentication
[xpc]: https://developer.apple.com/documentation/xpc
[launchservices]: https://developer.apple.com/documentation/coreservices/launch_services
[fsevents]: https://developer.apple.com/documentation/coreservices/file_system_events
[spotlight]: https://developer.apple.com/documentation/corespotlight
[cg]: https://developer.apple.com/documentation/coregraphics
[ct]: https://developer.apple.com/documentation/coretext
[ca]: https://developer.apple.com/documentation/quartzcore
[imageio]: https://developer.apple.com/documentation/imageio
[colorsync]: https://developer.apple.com/documentation/colorsync
[iosurface]: https://developer.apple.com/documentation/iosurface
[metal]: https://developer.apple.com/documentation/metal
[appkit]: https://developer.apple.com/documentation/appkit
[swiftui]: https://developer.apple.com/documentation/swiftui
[winwindows]: https://learn.microsoft.com/en-us/windows/win32/learnwin32/creating-a-window
[uia]: https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-uiautomationoverview
[dvt]: https://doronz88.github.io/pymobiledevice3/api/dvt/
[pymobiledevice]: https://github.com/doronz88/pymobiledevice3/tree/f6010ff51bc963884e688a2160db50b6b9a68b54
[simulator]: https://help.apple.com/xcode/mac/current/en.lproj/devfd9e28378.html
[idb]: https://github.com/facebook/idb/blob/56cd11d06e901b70650cbbe76bc12e41806a31e7/FBSimulatorControl/README.md
[cf-runloop]: https://github.com/apple-oss-distributions/CF/blob/dc54c6bb1c1e5e0b9486c1d26dd5bef110b20bf3/CFRunLoop.c
[cf-preferences]: https://github.com/apple-oss-distributions/CF/blob/dc54c6bb1c1e5e0b9486c1d26dd5bef110b20bf3/CFPreferences.c
[mach-message]: https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/mach/message.h
[mach-task]: https://github.com/llvm/llvm-project/blob/3878e680445418b076d7dd860f15171f22747751/lldb/tools/debugserver/source/MacOSX/MachTask.mm
[mach-exception]: https://github.com/llvm/llvm-project/blob/3878e680445418b076d7dd860f15171f22747751/lldb/tools/debugserver/source/MacOSX/MachException.cpp
[ptrauth]: https://github.com/llvm/llvm-project/blob/3878e680445418b076d7dd860f15171f22747751/clang/docs/PointerAuthentication.md
[unwind]: https://github.com/llvm/llvm-project/tree/3878e680445418b076d7dd860f15171f22747751/libunwind
