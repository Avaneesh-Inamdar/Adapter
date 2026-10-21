# Mach IPC milestone — 2026-10-21

Two unchanged Mach-O processes now perform a controlled Mach message exchange:
a server checks a service into a bootstrap namespace, receives a request with
send-once reply rights, and answers; a client looks the service up, sends, and
observes retirement. The same guest binaries also verify disposition, refcount,
timeout, LARGE-buffer and bootstrap failure semantics. This is the D2 slice of
the [OS services roadmap](roadmap.md); the broker is not a Mach kernel.

## Implementation

- Each worker owns a Mach name space with receive and send/send-once rights.
  `mach_task_self_` and `bootstrap_port` are exported data symbols holding
  stable names; every other port name is allocated per worker.
- `ADAPTER_MACH_BROKER` names an endpoint file written by `adapter-broker`
  (loopback TCP, a random 128-bit token, one JSON line). The broker owns service
  registration and forwards payloads between connections. Without the variable,
  the namespace is process-local and only self-send succeeds.
- A receive port has a global id `(connection << 32) | sequence`. A delivery to
  a port in the same worker is queued directly; otherwise it is sent to the
  broker with the destination global id, reply global id and disposition bits.
  The broker retires a global id when its receive port is destroyed or its
  connection closes.
- `mach_msg` accepts `MACH_SEND_MSG`, `MACH_RCV_MSG`, `MACH_RCV_LARGE` and
  `MACH_RCV_TIMEOUT`, and returns the Darwin result codes for invalid
  destination, right, reply, notify, type, option, size and timeout. Send size,
  receive size, header size and a 16 MiB message cap are validated.
- The receiver sees XNU's header inversion, not the sender's header:
  `msgh_local_port` is its own receive name, `msgh_remote_port` is a newly
  materialized send/send-once reply name, and `msgh_bits` carries the swapped
  normalized port types. A request sent with
  `MACH_MSGH_BITS(COPY_SEND, MAKE_SEND_ONCE)` therefore arrives with bits
  `0x1112`; a `MOVE_SEND_ONCE` reply arrives with bits `0x1200`.
- `MACH_RCV_LARGE` leaves an oversized queued message in place so the caller can
  retry with a larger buffer. Without it, the message is dropped before
  `MACH_RCV_TOO_LARGE` is returned.
- `mach_port_mod_refs` adjusts send and send-once counts; `MOVE_SEND` and
  `MOVE_SEND_ONCE` consume the right after a successful delivery, so a second
  use fails with `MACH_SEND_INVALID_RIGHT`. `mach_port_destroy` removes a
  receive right, marks its send rights dead and retires the global id.
- `bootstrap_check_in` allocates the receive right and registers the name;
  duplicate registration returns `BOOTSTRAP_SERVICE_ACTIVE` 1103.
  `bootstrap_look_up` returns `BOOTSTRAP_UNKNOWN_SERVICE` 1102 for absent
  names and a send right for registered ones. A missing endpoint file fails
  worker startup before guest entry with exit 125.
- Broker delivery pushes into a process-wide inbox and wakes a condition
  variable without taking guest Mach state, avoiding a reply/send deadlock.
  A dropped broker connection marks the namespace lost; pending lookups,
  sends and receives then fail instead of hanging.

## Consumers and assertions

`tests/fixtures/mach_server.c` checks in a service, writes a ready marker,
receives one request and asserts:

```text
server:checkin=0x0
server:recv=0x0
server:size=29
server:bits=0x1112
server:body_hex=70696e6700
server:payload=ping
server:reply=0x0
server:destroy=0x0
mach:server echo passed
```

The same fixture's `duplicate` mode returns success only when a second
check-in reports `0x44f` (1103).

`tests/fixtures/mach_client.c` looks the service up, allocates a reply port,
sends `ping` with message id `0x51`, receives with a five-second timeout and
asserts the reply header and body, waits for the server's destroyed-port marker,
then sends again to learn the destination is gone:

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

`tests/fixtures/mach_errors.c` runs against both local and broker-backed
namespaces and covers:

- 24-byte header size and invalid task/name/null-pointer returns
- self-send and receive bit inversion (`0x1112`/`0x1200`)
- stale send-once, `MOVE_SEND` consumption and wrong disposition
- `MACH_RCV_LARGE` retry versus non-LARGE drop followed by timeout
- duplicate check-in, bad bootstrap port, successful and absent lookup
- a registered service exchange with `0x1100` receive bits
- double destroy returning `KERN_INVALID_NAME` (15), and zero failures overall

The cross-process test uses a unique service name, asserts `0x44f` for a
duplicate check-in, and terminates the server and broker in `finally` blocks.
A negative test points `ADAPTER_MACH_BROKER` at a missing file and expects exit
125 with no guest stdout.

## Verification

The full verifier passed all 14 stages. The current run contains **118 distinct
tests: 25 Rust, nine archive and 84 integration**; all 84 integration tests also
pass against the release build. The Mach tests are:

```text
test_mach_failure_semantics_local
test_mach_failure_semantics_with_broker
test_mach_cross_process_echo_and_retire
test_mach_missing_broker_endpoint_fails_before_entry
```

The existing jq, ripgrep, dynamic-loading, thread and libc suites remain in the
same run. Evidence for the constants, header layout and receive-side rewriting
is in [the ABI evidence](reverse-engineering/mach-ipc-evidence.md).

## Boundaries

- **Not a Mach kernel.** Rights are bookkeeping in each worker plus liveness at
  the broker. There is no memory protection, scheduling, QoS, audit token or
  kernel-enforced right transfer between processes.
- **Simple messages only.** Complex messages, port descriptors, out-of-line
  memory, vouchers, trailers, port sets and notifications are rejected or
  absent. `msgh_voucher_port` is always zero on receive.
- **No port introspection.** `mach_port_extract_right`, `get_refs`, `peek`,
  `names`, `type`, `mach_port_request_notification` and strict-reply vouchers
  are unimplemented.
- **Bootstrap is a service table, not launchd.** Check-in creates the receive
  right rather than receiving one from launchd; there is no on-demand
  activation, `bootstrap_register`, `bootstrap_subset` or privileged namespace.
- **No process management.** `fork`, `exec` and `posix_spawn` remain explicit
  failures, so a broker started outside the test cannot spawn a service; D3
  must supply job spawning and lifecycle.
- **Loopback endpoint trust.** The endpoint file carries a random token and is
  read from the environment. It is a coordination channel for trusted local
  workers, not a security boundary against hostile local processes.
- **No reference-Mac comparison.** All observations are Windows-side; Darwin
  ordering for simultaneous senders, queue fairness and broker loss have no
  reference measurement.

## Reproduction

```powershell
cargo build --locked
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime -k mach -v
python tools/verify.py
```

Fixtures are generated under `tests/fixtures/generated/` and are not committed.
Broker endpoints are temporary files under the test's temporary directory.
