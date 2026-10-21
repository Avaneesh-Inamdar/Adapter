# launchd job and bootstrap evidence

Evidence for the D3 launchd slice. The facts below come from the pinned Xcode 27
XIP: the SDK's `launch.h`, `launchd.plist(5)` and `launchd(8)`. They are kept
separate from Adapter's own control plane, which is not launchd.

## Pinned artifact provenance

All paths are inside `Xcode.app` from `req files/Xcode_27.xip`
(SHA-256 `6a270c53a5a0c5e0ac78125342d44c3cfff2716ec390373cd5e30834d80a67c3`).
Extracted copies live under `target/artifacts/xcode27-sdk/launchd/`; each has a
provenance JSON recording the archive member and source hash.

| Artifact | Member in the XIP | SHA-256 | Size |
| --- | --- | --- | --- |
| `launch.h` | `MacOSX.sdk/usr/include/launch.h` | `0891454e2b1b568604c4becc2f56772155281d070aa773eceada279452146f03` | 15,709 |
| `launchd.plist.5` | `MacOSX.sdk/usr/share/man/man5/launchd.plist.5` | `1c5f5041c1d3492988bfa6f9dc6d969dfd072d20af2703c28d9a8f6ed6aaadcb` | 33,172 |
| `launchd.8` | `MacOSX.sdk/usr/share/man/man8/launchd.8` | `05a8aa5e6cc7353c0b8aacd1747ee99c2b60c6f88e4189b217ef60cb7fe3786f` | 3,077 |

## Facts read from the man pages

`launchd(8)` describes launchd as the system-wide and per-user daemon/agent
manager and says on-demand launch is preferred where possible
(`launchd.8:16`). It cannot be invoked directly and is started by the kernel as
the first process (`launchd.8:23-25`). This is why Adapter does not emulate
launchd by running it: the broker is a separate adapter-defined process.

`launchd.plist(5)` requires a `Label` string to identify the job
(`launchd.plist.5:124`). Plist files conventionally end in `.plist` and are
named after the label. The keys Adapter reads are described as follows:

- `Program` is the absolute path passed as the first argument of `execv`
  (`launchd.plist.5:181`). If missing, the first element of
  `ProgramArguments` is used, either absolute or relative to `_PATH_STDPATH`
  (`launchd.plist.5:197`).
- `ProgramArguments` is the argument vector passed to the job
  (`launchd.plist.5:197`).
- `KeepAlive` defaults to false, so only demand starts the job
  (`launchd.plist.5:269-274`). A true value keeps it alive; the key implies
  `RunAtLoad`.
- `RunAtLoad` defaults to false and launches the job once when the job is
  loaded (`launchd.plist.5:332-334`).
- `EnvironmentVariables` adds variables before running the job; values other
  than strings are ignored (`launchd.plist.5:373-383`).
- `StandardOutPath` and `StandardErrorPath` map the respective descriptors to
  a file, creating it if absent (`launchd.plist.5:491`, `507`).
- `MachServices` registers names in the Mach bootstrap namespace. Each value
  must be true or a dictionary for the service to be advertised
  (`launchd.plist.5:633-636`). Jobs dequeue messages with
  `xpc_connection_create_mach_service` or `bootstrap_check_in`
  (`launchd.plist.5:668-670`).
- `Sockets` activates launchd-provided file descriptors, retrieved with
  `launch_activate_socket` (`launchd.plist.5:672-680`).

The check-in requirement is explicit: a daemon or agent launched by launchd
**must** check in for every advertised `MachServices` name, using XPC or
`bootstrap_check_in` if it uses MIG or raw Mach (`launchd.plist.5:93-105`).
That is the contract Adapter reuses: the spawned guest calls the same
`bootstrap_check_in` that D2 already implements.

## Deprecated launch.h surface

`launch.h` states that its interfaces were documented only to let jobs obtain
socket descriptors, and that there are no replacements for other uses of
`launch_msg`, including submitting, removing, starting, stopping and listing
jobs (`launch.h:4-13`). `LAUNCH_KEY_SUBMITJOB` is still defined
(`launch.h:29`) and `launch_msg` is declared at `launch.h:403`, but both are
deprecated since macOS 10.10. `launch_activate_socket` remains the supported
socket API (`launch.h:197`).

Adapter therefore does not implement `launch_msg` or `launchctl` submission.
Jobs are supplied to the broker on its command line, which is an Adapter
control plane rather than an Apple ABI.

## How D3 maps onto these facts

| launchd behavior | D3 implementation | Divergence |
| --- | --- | --- |
| Job loaded from a plist | `adapter-broker --job FILE` reads an XML or binary plist at startup | No LaunchDaemons/LaunchAgents directories and no `launchctl` submission |
| `Label` required and unique | Required, 1-255 bytes, duplicate labels rejected before the endpoint is published | An empty label is rejected by the adapter; Darwin does not document the bound |
| `Program`/`ProgramArguments` | `Program` is passed to `adapter-run`; `ProgramArguments[0]` is dropped because `adapter-run` supplies argv[0] | Relative `ProgramArguments[0]` is not resolved through `_PATH_STDPATH`; `Program` need not be absolute |
| Demand launch | A `bootstrap_look_up` for a registered, inactive service spawns the job; a second lookup after exit respawns it | Job exit is not throttled; there is no `KeepAlive`, launch-on-socket or event activation |
| `RunAtLoad` | The broker activates those jobs after publishing the endpoint | No boot/login domains; all jobs live in one broker namespace |
| `MachServices` | Names are reserved at broker startup; lookup waits up to 15 seconds for the spawned job to check in | The receive right is created by `bootstrap_check_in` in the guest, not pre-created by launchd; a request sent before check-in is not queued |
| `EnvironmentVariables` | Applied to the child process before spawn | Only string values are accepted; non-strings fail the job load rather than being ignored |
| `StandardOutPath`/`StandardErrorPath` | Files opened in append mode; `/C/...` guest paths map to `C:\...`; absent means null | No umask, ownership or `StandardInPath` handling |
| Check-in | The child inherits `ADAPTER_MACH_BROKER` and uses D2 `bootstrap_check_in` | No privileged namespace, `bootstrap_subset`, `bootstrap_register` or per-user sessions |

Unsupported plist keys fail the broker load rather than being silently
accepted. This is stricter than launchd, which ignores or implements many more
keys; it prevents a plist from appearing supported when its semantics are not.

## What D3 deliberately does not implement

- `launchd` as a process, PID 1 behavior, bootstrapping, or `launchctl`.
- `launch_msg` submission/removal/start/stop, `launch_activate_socket`,
  `Sockets`, `inetdCompatibility`, launch events and XPC event streams.
- `KeepAlive` (including `SuccessfulExit` and `Crashed` conditions),
  throttling, `StartInterval`/`StartCalendarInterval`, `ProcessType`,
  `ThrottleInterval`, watchdog, jetsam and `ExitTimeOut` semantics.
- `UserName`/`GroupName`, `Umask`, `RootDirectory`, `WorkingDirectory`,
  `StandardInPath`, `SessionCreate`, `LimitLoadToSessionType` and privacy/TCC.
- `MachServices` sub-keys `ResetAtClose` and `HideUntilCheckIn`, and
  launchd-precreated receive rights. The adapter's service name assignment is
  owned by the broker, and check-in creates the receive right as in D2.
- XPC services and bundled `.xpc` discovery (D4).
