# Delivery phases

All 36 requested work items. Implemented slices do not imply complete subsystem support.

Status describes the listed slice. It is not a percentage of macOS or Xcode.

[Phase graph](progress/phases.svg) · [ABI contracts](abi/README.md) · [Pinned upstream sources and reuse limits](upstream-sources.md)

## Architecture decision

Windows x64 runs Intel guests directly; keep it as the regression baseline.
Native Windows ARM64 port, conditional on an available ARM64 host and ABI probes.
ARM64-to-x64 dynarec is a separate backend if the x64-host requirement is retained.
Measure guest atomic ordering. Apple's Rosetta TSO control is not assumed available on Windows ARM hardware.

## P0: Evidence, contracts and debugging

Status: **verified-slice**. Dependencies: none.

| # | Work item | Status | Remaining work / boundary |
| ---: | --- | --- | --- |
| 1 | Real macOS reference execution | reference-verified | Hosted Intel Mac reference executed successfully (run39667131995); compressed native capture, OS/build and exact executable hash retained. Physical device/GPU/desktop traces remain separate. |
| 2 | Differential harness: thousands of probes | verified-slice | 4096 same-artifact cases across eight libc families match in debug and release after correcting strtol no-conversion errno and timegm pre-1900 rejection (236 of 4096 differed before the fix). Expand to other subsystems and programs. |
| 4 | Guest crash debugging and single-step | verified-slice | Nine Windows LLDB tests cover genuine guest UD2, register/disassembly/backtrace capture, relocated symbols, VM breakpoints, instruction step and timeout cleanup, in debug and release; the release hook needed a unique body to survive identical-code folding. Darwin debugserver APIs remain P5. |
| 5 | Strict subsystem ABI contracts | implemented-contracts | Seven versioned subsystem contracts and automated layout/path/symbol-drift checks exist. Extend contracts and native reference coverage with every new subsystem. |

**Exit gate:** Same-artifact Mac/Adapter comparisons with complete case IDs; versioned ABI contracts; guest-PC, registers and instruction-step evidence.

## P1: Loaders and language runtimes

Status: **implemented-slices**. Dependencies: P0.

| # | Work item | Status | Remaining work / boundary |
| ---: | --- | --- | --- |
| 7 | System image and dyld shared cache | not-started | Acquire identified user-supplied image, extract/rebase caches and subcaches, resolve shared-cache fixups and image lifecycle. Existing XIP extraction is not cache loading. |
| 8 | Apple-ABI Objective-C runtime | verified-slice | Raw-isa MRC classes/metaclasses, selectors, inheritance, msgSend/Super2, load/initialize, startup categories and absolute/relative method lists across classic, chained and relative fixup forms. Protocols/properties, ARC, direct-selector lists, exceptions and dynamic registration remain. |
| 9 | Swift guest runtime | not-started | Apple Mach-O Swift metadata/refcounting/concurrency/reflection and ObjC interop. Native Windows Swift alone does not satisfy this ABI. |
| 10 | Darwin compact unwind and DWARF | not-started | Loaded-image unwind lookup, personalities and C++/ObjC exceptions across guest/host frames; distinguish Windows SEH. |
| 11 | Mach VM emulation | not-started | vm_*/mach_vm_* address spaces, protection, region queries, task rights, sharing and exact errors over host VM. |

**Exit gate:** Measured shared-cache images, ObjC/Swift consumers, cross-image unwinding and Mach VM error/lifetime probes.

## P2: System services and Foundation

Status: **implemented-slices**. Dependencies: P1.

| # | Work item | Status | Remaining work / boundary |
| ---: | --- | --- | --- |
| 14 | libdispatch / GCD | not-started | Assess upstream Windows backend, bridge guest Blocks/calling convention, queues, QoS and Mach dispatch sources. |
| 15 | Full CoreFoundation and Foundation | implemented-slice | Current CF strings only; add collections, constant objects, resources, bundles, NSObject lifecycle and toll-free bridging from consumers. |
| 16 | CFRunLoop on Mach ports | not-started | Modes, sources, observers, timers, wakeups and Mach-port callbacks, with reference ordering. |
| 17 | Complex Mach messages | implemented-slice | Simple broker messages exist. Add OOL/port descriptors, rights transfer, copy/deallocate rules and partial-failure cleanup. |
| 18 | XPC | implemented-slice | Typed broker transport exists; Apple interoperability, richer objects, GCD scheduling, peer validation and entitlements remain. |
| 19 | Real launchd semantics | implemented-slice | Job plist/demand-spawn subset exists. Add domains, KeepAlive, supervision/throttling, socket activation, inherited rights and launchctl behavior. |
| 20 | fork / exec / posix_spawn | not-started | Start with spawn and argument/environment/descriptor inheritance; model fork snapshots and atfork separately. |
| 21 | Signals and syscall personality | not-started | Guest signal numbers/contexts, masking, alternate stacks, restart and direct syscall interception. |
| 22 | IOKit / sysctl / proc_info | not-started | Target-specific device/process facts and guest object/handle ownership; no host-handle substitution. |
| 23 | macOS service daemons | not-started | cfprefsd, notifyd, distnoted, LaunchServices, securityd/trustd and opendirectoryd contracts, persistence and reference conversations. |
| 24 | FSEvents and Spotlight | not-started | Persistent event IDs, rescan/drop semantics, indexing and query behavior; Windows directory notifications are only a backend. |

**Exit gate:** Reference-matched queues/runloops, complex IPC, process behavior and real service consumers; preserve existing broker regressions.

## P3: Graphics and desktop UI

Status: **not-started**. Dependencies: P2.

| # | Work item | Status | Remaining work / boundary |
| ---: | --- | --- | --- |
| 25 | CoreGraphics / CoreText / CoreAnimation / ImageIO / ColorSync | not-started | Geometry, fonts/text metrics, image/color handling and compositor lifetimes with actual consumers. |
| 26 | IOSurface | not-started | Pixel layouts, process sharing, rights, synchronization and host GPU resource interop. |
| 27 | Metal to D3D12; AIR to DXIL | not-started | Separate resource/queue/synchronization API contracts and shader translation. DXC emits DXIL but does not decode AIR. |
| 28 | AppKit | not-started | Windows/events, responders, menus, resources, text and accessibility under the Apple guest object ABI. |
| 29 | SwiftUI | not-started | Depends on guest Swift/ObjC/Foundation, rendering and window lifecycle; no complete current Apple implementation available in the reviewed public sources. |
| 30 | WindowServer replacement | not-started | Win32 window/input/IME/focus/pasteboard/drag-drop/accessibility backend and guest-facing service contract. |

**Exit gate:** NSWindow hello-world with observed drawing, input, pasteboard and orderly teardown; resource/shader tests before claiming graphics coverage.

## P4: ARM64 and arm64e execution

Status: **not-started**. Dependencies: P0, P1.

| # | Work item | Status | Remaining work / boundary |
| ---: | --- | --- | --- |
| 3 | ARM64 backend decision | decision-recorded | Prefer native Windows ARM64 when hardware is available; retain dynarec as a separately scoped x64-host backend. Neither is implemented. |
| 12 | 16K guest pages and ARM memory ordering | not-started | Preserve guest alignment/protection/query semantics independently of host page granularity; run atomic litmus tests before choosing ordering adaptations. |
| 13 | arm64e pointer authentication | not-started | Keys/discriminators/address diversity, signed returns and authenticated fixups. Plain ARM64 or stripping pointer bits is insufficient. |

**Exit gate:** Pinned ARM64 CLI and architecture litmus probes; 16K guest-page semantics; authenticated-pointer tests separately from plain ARM64.

## P5: Xcode integration

Status: **not-started**. Dependencies: P2, P3, P4.

| # | Work item | Status | Remaining work / boundary |
| ---: | --- | --- | --- |
| 6 | Binary pass/fail milestones | not-started | NSWindow hello-world -> xcodebuild -> Xcode welcome window -> open project -> build project. API registrations cannot satisfy a milestone. |
| 31 | LLDB/debugserver Mach primitives | not-started | Exception ports, task_for_pid, thread_get_state, suspend/resume and memory access. Host LLDB debugging is a separate P0 capability. |
| 32 | codesign / Security / keychain | not-started | Signature/trust parsing, guest signing/entitlement policy and persistent keychain behavior with test fixtures. |
| 33 | xcrun / xcode-select / SDK paths | not-started | Developer directory state, tool/SDK discovery, platform/version queries and actual subprocess execution. |

**Exit gate:** Pass each of M0 through M4 with the actual named executable and a retained trace/output artifact.

## P6: Instruments, devices and Simulator

Status: **not-started**. Dependencies: P5.

| # | Work item | Status | Remaining work / boundary |
| ---: | --- | --- | --- |
| 34 | Instruments and DTX services | not-started | Pin protocol implementation, capture real service conversations and bridge profiling capabilities; protocol code is not the Instruments app. |
| 35 | Physical devices / USB / usbmuxd / CoreDevice / pairing | external-gate | Windows transport/drivers, trust records, developer mode and service versions require an actual device acceptance run. |
| 36 | iOS Simulator / CoreSimulator / iOS runtime | deferred | Defer until earlier Xcode gates pass; automation clients do not replace CoreSimulator or iOS frameworks. |

**Exit gate:** Measured DTX/device pairing conversations and actual device tests; Simulator remains deferred until prior Xcode gates pass.

## Binary acceptance gates

A gate passes only with the actual named guest program and retained execution evidence. Registered API counts and substitute host programs do not pass a gate.

| Gate | Result | Acceptance |
| --- | --- | --- |
| M0: NSWindow hello-world | NOT PASSED | Actual guest NSWindow appears, draws, receives input and closes cleanly. |
| M1: xcodebuild | NOT PASSED | Identified Apple xcodebuild performs version/SDK queries through Adapter; host compiler invocation is not a substitute. |
| M2: Xcode welcome window | NOT PASSED | Identified Xcode launcher reaches the interactive welcome window with service/runtime logs. |
| M3: Open project | NOT PASSED | Open a pinned small project and reach editor/project state without a fatal unsupported call. |
| M4: Build project | NOT PASSED | Build the pinned project, retain the output artifact and compare command/results with the Mac reference. |

Edit [phases.json](phases.json), then run `python tools/check_contracts.py` and `python tools/update_phases.py --write`. The default command detects stale output.
