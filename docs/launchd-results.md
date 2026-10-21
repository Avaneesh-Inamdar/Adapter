# launchd bootstrap milestone — 2026-10-21

The broker now starts a registered launchd-style job when a guest looks up one
of the job's Mach services, and an unchanged Mach-O client reaches it. Jobs can
also be started when the broker loads. This is the D3 slice of the
[OS services roadmap](roadmap.md); it is an Adapter control plane, not launchd.

## Implementation

- `adapter-broker --endpoint FILE [--job PLIST ...]` loads one or more property
  lists before binding its socket. Duplicate labels or a service advertised by
  two jobs fail startup. If any job is present, `adapter-run.exe` must sit next
  to the broker.
- [src/broker/job.rs](../src/broker/job.rs) accepts exactly these keys:
  `Label`, `Program`, `ProgramArguments`, `MachServices`, `EnvironmentVariables`,
  `StandardOutPath`, `StandardErrorPath` and `RunAtLoad`. Any other key is an
  error (`unsupported launchd key X`), which prevents a plist from looking
  supported when its semantics are not. `Label` is required and unique;
  `Program` or `ProgramArguments[0]` supplies the executable. When both exist,
  `ProgramArguments[0]` is dropped because `adapter-run` supplies argv[0].
- `MachServices` values must be true or a dictionary; false entries are not
  advertised. `EnvironmentVariables` accepts string values only.
  `StandardOutPath`/`StandardErrorPath` are opened in append mode.
- The broker records a `JobState` of `Idle`, `Starting`, `Running { pid }` or
  `Failed`. Activation spawns
  `adapter-run.exe <Program> -- <arguments>` with `ADAPTER_MACH_BROKER` set to
  the broker's endpoint, the job's environment, and the configured logs or a
  null stdin/stdout/stderr. A watcher thread returns the job to `Idle` when the
  child exits.
- A lookup for an advertised service with no live owner starts the job if it is
  `Idle` or `Failed`, then waits on the broker's condition variable for check-in.
  The wait is bounded at 15 seconds; an exit during the wait allows one
  respawn. Success returns the check-in receive port to the client. Timeout,
  a second failure, or a job that never checks in returns
  `BOOTSTRAP_UNKNOWN_SERVICE` (0x44e).
- `RunAtLoad` jobs are activated immediately after the endpoint file is
  published, before the broker accepts connections. A job that fails to spawn
  is recorded as `Failed`; its next lookup tries once more and then reports
  `0x44e`.
- If the broker exits, its TCP connections close. Running jobs are not killed
  or signalled: they observe the lost namespace on their next Mach call, as in
  D2. There is no KeepAlive, throttling, socket activation or process group.

## Consumers and assertions

The D3 tests register the existing unchanged `mach_server` fixture as a job and
reach it with the unchanged `mach_client`. Four integration tests cover the
gate:

- `test_broker_spawns_registered_job_on_demand` starts a broker with one job and
  no `RunAtLoad`; the first client triggers check-in, request/reply and
  retirement, the server log records a full pass, and a second client causes a
  second spawn and a second `mach:server echo passed` line. This asserts both
  demand activation and respawn after exit.
- `test_broker_runs_job_at_load` starts a job with `RunAtLoad` true, observes
  its ready marker before any lookup, then performs the same client exchange.
- `test_broker_job_spawn_failure_returns_unknown_service` registers a program
  that does not exist and asserts the client's lookup returns `0x44e` and the
  client exits 10.
- `test_broker_rejects_unsupported_launchd_key` supplies `KeepAlive`, expects
  broker exit 125 with `unsupported launchd key KeepAlive`, and asserts no
  endpoint file was published.

The successful client output is identical to D2:

```text
client:lookup=0x0
client:send=0x0
client:recv=0x0
client:bits=0x1200
client:payload=pong:ping
client:retired_seen=1
client:dead_send=0x10000003
mach:cross-process echo passed
```

Four broker-free unit tests in `src/broker/job.rs` cover key parsing,
`ProgramArguments`-only jobs, rejection of missing or invalid fields, and guest
drive-path mapping.

## Verification

The full verifier passed all 14 stages. The current run contains **126 distinct
tests: 29 Rust, nine archive and 88 integration**; all 88 integration tests also
pass against the release build. The new integration tests are:

```text
test_broker_spawns_registered_job_on_demand
test_broker_runs_job_at_load
test_broker_job_spawn_failure_returns_unknown_service
test_broker_rejects_unsupported_launchd_key
```

All prior jq, ripgrep, dynamic-loading, thread, libc and Mach suites remain in
the same run. The launchd keys, check-in requirement and deprecated submission
surface are recorded in
[the launchd evidence](reverse-engineering/launchd-bootstrap-evidence.md).

## Boundaries

- **Not launchd.** There is no PID 1, boot domain, per-user domain, `launchctl`,
  `launch_msg`, job removal or service discovery from LaunchDaemons directories.
  Jobs exist only in the broker process that was given their plists.
- **Demand is lookup-driven.** Only a Mach service lookup activates a job.
  Sockets, XPC event streams and calendar/interval triggers are absent.
- **No supervision.** `KeepAlive`, throttling, signals, timeouts and crash
  classification are absent. The broker observes child exit but does not
  restart jobs on its own.
- **Check-in creates the port.** launchd pre-creates service receive rights;
  here the job's `bootstrap_check_in` creates and registers the receive right
  after spawn. Messages sent before check-in are not queued by the broker.
- **One namespace.** All jobs share the broker's flat service table; there is no
  privileged domain, `bootstrap_subset` or per-user isolation.
- **Path mapping only.** `/C/...` and `/D/...` program and log paths map to
  Windows drives; `Program` is not required to be absolute and relative
  `ProgramArguments[0]` is not resolved through `_PATH_STDPATH`.
- **Trusted local control plane.** Job plists and the endpoint are read from the
  local filesystem. The broker is not a sandbox and does not authenticate job
  authors.
- **No reference-Mac comparison.** Demand timing, respawn ordering and process
  teardown have only been observed on Windows.

## Reproduction

```powershell
cargo build --locked
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime -k broker -v
python tools/verify.py
```

A minimal job plist for the fixture (the test generates the same structure with
absolute paths):

```xml
<plist version="1.0"><dict>
  <key>Label</key><string>com.example.echo</string>
  <key>Program</key><string>/C/Adapter/tests/fixtures/generated/mach_server</string>
  <key>ProgramArguments</key><array>
    <string>mach_server</string><string>echo</string>
    <string>com.example.echo</string>
    <string>/C/Adapter/ready</string><string>/C/Adapter/retired</string>
  </array>
  <key>MachServices</key><dict><key>com.example.echo</key><true/></dict>
  <key>StandardOutPath</key><string>/C/Adapter/server.log</string>
</dict></plist>
```

Start the broker with `--job job.plist`; a client that looks up
`com.example.echo` starts the job.
