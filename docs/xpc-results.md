# XPC milestone — 2026-10-22

An unchanged Mach-O `libxpc` server and client now exchange a typed dictionary
request and reply through the loopback broker, and a missing service surfaces
as `XPC_ERROR_CONNECTION_INVALID`. The server can also be started on demand by
the D3 job path. This is the D4 slice of the [OS services roadmap](roadmap.md):
an adapter-defined XPC transport, not Apple's private wire protocol.

## Implementation

- The loader resolves `/usr/lib/system/libxpc.dylib` to
  [the XPC adapter](../src/darwin/xpc.rs). It exports 77 `_xpc_*` functions and
  22 `__xpc_*` data symbols: 17 type markers (including endpoint),
  `__xpc_bool_true`, `__xpc_bool_false`, the two connection error dictionaries
  and `__xpc_error_key_description`.
- Type constants are addresses of exported marker structs, as the pinned
  headers define them, and the error constants are addresses of static error
  dictionaries; the adapter does not export pointer variables. `xpc_get_type`
  returns the marker address for the object's actual kind, and a null or unknown
  handle is typed as null.
- Objects live in a runtime registry. Each live object is a unique heap key with
  an atomic reference count; strings, data, arrays and dictionaries own their
  contents, containers retain their children, and a released child remains
  reachable while a container still references it. `xpc_retain`, `xpc_release`,
  `xpc_copy`, `xpc_equal`, `xpc_hash` and `xpc_copy_description` operate on the
  same model. Static null/bool/error objects are singletons and are never freed.
- `xpc_connection_create_mach_service` with the listener flag performs a
  bootstrap check-in through the D2/D3 Mach service table when the connection is
  activated. A non-listener connection connects through the broker by service
  name. The broker pairs the two workers with one route id each, records a
  bidirectional link, and delivers an accept event carrying the server's route.
  Activating an accepted peer does not reconnect.
- `xpc_connection_set_event_handler` copies the block; the connection dispatcher
  invokes it with `xpc_connection_get_type`-style object arguments. A single
  process-wide dispatcher thread serves every connection and preserves per-
  connection order; `set_target_queue` is recorded but does not select a guest
  GCD queue. `xpc_connection_suspend`/`resume` balance, and
  `xpc_connection_send_message_with_reply` delivers the reply through its block
  while the `_sync` form blocks on a condition variable.
- `xpc_dictionary_create_reply` creates at most one reply for a received
  message, and `xpc_dictionary_get_remote_connection` returns the connection the
  reply must be sent on. Cancel releases the connection's Mach service port,
  closes its broker route, delivers the final `XPC_ERROR_CONNECTION_INVALID`
  event, and causes the peer to receive `XPC_ERROR_CONNECTION_INTERRUPTED`.
- Messages are serialized when sent; later mutation of the sender's object does
  not change the message. The wire format is adapter-defined: little-endian
  magic `0x5850`, version 1, tags for null, bool, int64, uint64, double, string,
  data, array, dictionary and error, with a depth cap of 64, a 16 MiB total and
  value cap, a 1 MiB string/key cap and cycle rejection. The broker forwards
  only `XPC_MESSAGE` and `XPC_REPLY` frames between routes that belong to the
  same link; anything else produces an invalid-connection notice.
- Public boolean-returning adapters return `i32` 0/1. Rust `bool` leaves the
  upper `%eax` bytes unspecified in the SysV ABI, and a C caller that reads the
  full `int` would observe garbage; the release-build verifier caught this and
  the adapters were corrected.

## Consumers and assertions

`tests/fixtures/xpc_server.c` creates a listener for the service named in
argv[1], writes a ready marker, and on accept sets the peer handler:

```text
server:listener=1 name=<service>
server:accept=<service>
server:message=ping value=41 count=4
server:reply=1
server:peer_invalid=1
server:done=1 invalid=1
```

`tests/fixtures/xpc_client.c` connects, checks that the reply is a dictionary,
and validates every value type in a single request/reply exchange:

```text
client:reply_type=dictionary
client:echo=ping
client:pong=42 ok=1 ratio=2.5 count=6
client:list=2 first=alpha second=7
client:nested=fixture
client:valid=1
client:invalid=1
xpc:client reply passed
```

`tests/fixtures/xpc_missing.c` connects to an absent service and requires the
invalid event:

```text
missing:invalid=1
xpc:missing service passed
```

Three integration tests cover the gate:

- `test_xpc_dictionary_roundtrip` runs a broker, starts the server manually,
  performs the request/reply exchange, and asserts the server's full log
  including peer interruption after the client cancels.
- `test_broker_spawns_xpc_listener_on_demand` registers the same server as a
  launchd-style job and lets the client's lookup start it; the client sees the
  identical reply and the server log records check-in, accept, message and
  termination.
- `test_xpc_missing_service_is_invalid` looks up an unregistered service and
  asserts the invalid-connection event and exit 0.

## Verification

The verifier at this milestone passed all 14 stages with **132 distinct tests:
31 Rust, nine archive and 92 integration**; all 92 integration tests also passed
against the release build. Later slices grew the suites; the
[current summary](test-results/summary.json) holds the final counts. All prior jq, ripgrep, dynamic-loading, thread,
libc, Mach and launchd suites remain in the same run. The pinned headers,
symbol inventory and connection contract are recorded in
[the XPC evidence](reverse-engineering/xpc-evidence.md).

## Boundaries

- **Not Apple's wire format.** Adapter workers can talk to each other, not to a
  real macOS process or `launchd`. A compatible serialization and Mach-based
  bootstrap transport would be required for interoperability.
- **No `xpc_main` or GCD.** Handlers run on Adapter's dispatcher thread; target
  queues, dispatch sources, transactions, activity scheduling and `xpc_main`
  are absent. Ordering, QoS and coalescing differ from Darwin.
- **Value subset.** Date, UUID, file descriptor, shared-memory, endpoint and
  rich-error objects are not implemented; only the two connection errors exist.
  File descriptors cannot be transferred, and there is no out-of-line data.
- **No peer validation or sandbox.** The adapter does not verify code signing,
  entitlements, audit tokens or sandbox profiles, and it cannot grant or deny
  Mach entitlements. Its service namespace is flat and trusted-local, like the
  broker; it is not a security boundary. xpc_connection_set_peer_*_requirement,
  `xpc_connection_get_*` audit accessors and entitlement checks are absent.
- **One namespace, no privilege.** A listener registers in the broker's flat
  service table; there is no per-user or system domain and no privileged flag
  behavior.
- **Bounded values.** Payloads over 16 MiB, strings or keys over 1 MiB, nesting
  over 64 levels and cyclic containers are rejected locally; partial delivery is
  not attempted.
- **No reference-Mac comparison.** Message ordering, event delivery and failure
  timing have only been observed on Windows.

## Reproduction

```powershell
cargo build --locked
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime -k xpc -v
python tools/verify.py
```

Fixtures are generated under `tests/fixtures/generated/` and are not committed.
The server and client are ordinary Mach-O executables cross-linked with
`-lxpc -lSystem`; the adapter resolves the library install name.
