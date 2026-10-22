# Item dispositions: host-implementable work versus Apple-input gates

Every open item in [the phase inventory](phases.json) falls into one of two
classes. This page records which, with the exact input each gate would need, so
"not started" is never confused with "blocked".

- **A - implementable on this host.** A slice can be built, tested and compared
  against a hosted macOS reference using artifacts already obtainable here: the
  cross-compiled fixtures in `tests/fixtures/generated`, the pinned jq and
  ripgrep releases, the supplied Xcode XIP extracts, or a fixture built by the
  hosted `macos-15-intel` runner and committed by hash.
- **B - Apple-input gate.** The behaviour lives in a proprietary Apple artifact
  that this repository may not redistribute, and Apple publishes no
  specification that would let it be reimplemented faithfully. A hosted macOS VM
  can *run* such an artifact, but Adapter must execute the guest on Windows, so
  the artifact would have to exist on this host in a redistributable form. It
  does not, and a same-named substitute would not establish compatibility.

## A - implementable here (next slice named)

| Item | Next concrete slice |
| --- | --- |
| 7. System image and dyld shared cache | Parse and rebase a user-supplied cache from the XIP extracts already pinned. |
| 9. Swift guest runtime | Build Swift fixtures on the hosted Mac runner, commit them by hash, then load metadata/refcounting. |
| 10. Darwin compact unwind and DWARF | `__unwind_info` and FDE lookup over guest images, distinct from Windows SEH. |

| 12. 16K guest pages and ARM memory ordering | Page-granular placement and ordering tests once an ARM64 guest exists. |
| 13. arm64e pointer authentication | Documented QARMA-based signing with synthetic arm64e fixtures. |
| 14. libdispatch / GCD | Queue, semaphore and block-bridge slice over host threads. |
| 16. CFRunLoop on Mach ports | Modes, timers, sources and wakeups over the existing Mach message slice. |
| 20. fork / exec / posix_spawn | `posix_spawn` and `execve` over a re-entered Adapter instance. |
| 21. Signals and syscall personality | Delivery, `sigaction`/`sigprocmask` and Darwin errno/personality probing. |
| 22. IOKit / sysctl / proc_info | `sysctlbyname` coverage and `proc_info` answers for the running guest. |
| 24. FSEvents and Spotlight | `FSEventStream` over `ReadDirectoryChangesW`, with an event-order contract. |
| 26. IOSurface | Adapter-owned shared buffers with handle, plane and lock semantics. |
| 31. LLDB/debugserver Mach primitives | A gdb-remote stub over the Windows debugging API plus the existing crash-path tooling. |
| 32. codesign / Security / keychain | Signature parsing and Apple trust anchors; keychain mapped to host storage. |
| 33. xcrun / xcode-select / SDK paths | Tool lookup and SDK path answers over the already-extracted Xcode contents. |

## B - Apple-input gates

| Item | Required input that this host lacks | What is recorded instead |
| --- | --- | --- |
| 6. Binary pass/fail milestones | AppKit plus Xcode's own GUI binaries: M0 NSWindow hello-world, M1 `xcodebuild`, M2 the Xcode welcome window, M3 open project, M4 build project. | [Phases](phases.md) keeps the gate definitions and pass criteria; no gate can pass without the AppKit framework. |
| 23. macOS service daemons | `cfprefsd`, `notifyd`, `distnoted`, `LaunchServices`, `securityd`/`trustd`, `opendirectoryd` daemon binaries; not present in the Xcode XIP and not redistributable. | [Launchd](launchd-results.md) and [XPC](xpc-results.md) slices cover the client side against adapter-owned services. |
| 25. CoreGraphics / CoreText / CoreAnimation / ImageIO / ColorSync | Apple framework images plus their rendering and text-layout behaviour, which Apple documents only at API level. | [Limitations](limitations.md) records the absent framework surface. |
| 27. Metal to D3D12; AIR to DXIL | Apple's Metal front end that emits AIR; DXC has no AIR front end and no AIR-to-DXIL implementation was identified. | [Upstream research](upstream-sources.md) records the search. |
| 28. AppKit | The AppKit framework image and the Cocoa text/responder stack it depends on. | As above. |
| 29. SwiftUI | Apple's shipping SwiftUI binary; the Swift compiler repository does not contain it. | As above. |
| 30. WindowServer replacement | WindowServer behaviour and its private protocols, observable only from a running Mac. | As above. |
| 34. Instruments and DTX services | `Instruments.app` and its DTX peer implementation. | [Mach IPC](mach-ipc-results.md) covers the message layer DTX would use. |
| 35. Physical devices / USB / usbmuxd / CoreDevice / pairing | An attached device and its trust records for an acceptance run. | [Limitations](limitations.md); status `external-gate`. |

Item 36 (iOS Simulator / CoreSimulator / iOS runtime) stays `deferred`: it
depends on the B-class inputs above and on the P3 stack.

## Rule

An item moves out of `not-started` only when its slice has a consumer, a test and
recorded evidence, or when it is assigned to one of the classes above with the
input named. A gate is not a claim of compatibility, and a substitute with the
same API names is never counted as the Apple artifact.
