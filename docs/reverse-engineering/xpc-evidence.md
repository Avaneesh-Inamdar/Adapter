# XPC header and symbol evidence

Evidence for the D4 XPC slice. The facts below come from the pinned Xcode 27 SDK
in the XIP. They define the public object model and connection contract that
Adapter targets; they do not describe Apple's private XPC wire format, which
Adapter does not claim to implement.

## Pinned artifact provenance

All paths are inside `Xcode.app` from `req files/Xcode_27.xip`
(SHA-256 `6a270c53a5a0c5e0ac78125342d44c3cfff2716ec390373cd5e30834d80a67c3`).
Extracted copies live under `target/artifacts/xcode27-sdk/xpc/`; each has a
provenance JSON recording the archive member and source hash.

| Artifact | Member in the XIP | SHA-256 | Size |
| --- | --- | --- | --- |
| `xpc.h` | `MacOSX.sdk/usr/include/xpc/xpc.h` | `80ab62828eae4081288d496353391f4c117a0d9a68bd6d7ed495937a57d89982` | 85,100 |
| `connection.h` | `iPhoneOS.sdk/usr/include/xpc/connection.h` | `fee99c719a44ff57f5f3bd9e3e2d2b5518d093a476c0df9d4fbcc6a358750d51` | 43,953 |
| `endpoint.h` | `MacOSX.sdk/usr/include/xpc/endpoint.h` | `360df5662cbc16a7ff5628a2472f2f58a4f509621035068226a1ba5e22c32499` | 715 |
| `Block.h` | `MacOSX.sdk/usr/include/Block.h` | `c1f75e3d4d3b06d7ffbd50697666c874236d269e30307e303609d71f2269bdd9` | 1,541 |
| `libxpc.tbd` | `MacOSX.sdk/usr/lib/system/libxpc.tbd` | `cff42e2a92929e4ae0bdd06db2ead82292a7dcce1e2240564478ddb9f7e321f5` | 32,518 |

The `MacOSX.sdk` copy of `xpc/connection.h` is an XIP filesystem placeholder
(a `NULLcanary` member). The first data-bearing carrier in archive order is the
iPhoneOS SDK copy, so that copy is pinned instead; the placeholder is retained
under `target/artifacts/xcode27-sdk/placeholders/` with its metadata. The two
platform headers describe the same public API. `libxpc.tbd` declares install
name `/usr/lib/system/libxpc.dylib` and version 3298.1.1 and lists 527 unique
`_xpc_*` symbols, including 32 `_xpc_type_*` markers, the two
`_xpc_error_connection_*` globals, `_xpc_bool_true`/`_xpc_bool_false`,
`_xpc_null`, 65 `xpc_connection_*` entry points and 53 `xpc_dictionary_*`
entry points.

## Object model

`xpc_object_t` is `void *` (`xpc.h:82`) and `xpc_type_t` is a pointer to an
opaque `_xpc_type_s` (`xpc.h:46`). The type constants are addresses of
exported marker structs, not distinct pointer values supplied by a function:
`XPC_TYPE_CONNECTION` is `&_xpc_type_connection` (`xpc.h:110`), with the same
pattern for null, bool, int64, uint64, double, date, data, string, uuid, fd,
shmem, array, dictionary, error and rich error (`xpc.h:150-335`).

The error constants follow the same rule. `XPC_ERROR_CONNECTION_INVALID` is a
macro over the address of the exported `_xpc_error_connection_invalid`
dictionary (`connection.h:53-57`); `XPC_ERROR_CONNECTION_INTERRUPTED` is the
address of `_xpc_error_connection_interrupted` (`connection.h:34-38`). The
header builds these with `XPC_GLOBAL_OBJECT`, which is `&(object)` for
non-ObjC builds (`xpc.h:84`). Adapter therefore exports static marker structs
and static error dictionaries and returns their addresses; it must not export
pointer variables.

`xpc_bool_create` returns one of the global singletons: `XPC_BOOL_TRUE` and
`XPC_BOOL_FALSE` are addresses of `_xpc_bool_true`/`_xpc_bool_false`
(`xpc.h:169-182`). Retain/release/get_type/copy/equal/hash/copy_description are
declared at `xpc.h:381-541`.

## Connection contract

- `xpc_connection_create_mach_service(name, targetq, flags)` takes
  `XPC_CONNECTION_MACH_SERVICE_LISTENER` (1) for a listener and
  `XPC_CONNECTION_MACH_SERVICE_PRIVILEGED` (1 << 1)
  (`connection.h:104-114`, `205`). The header states that a created connection
  is inactive and must be activated before it sends or receives
  (`connection.h:199-206`); it is therefore legal to create a connection before
  the peer or service exists.
- A listener receives each accepted peer as a new connection object in its
  event handler; the receiver must set a handler and activate/resume it, or
  cancel it to reject (`connection.h:310-326`).
- `xpc_connection_activate` is the current form; `xpc_connection_resume` is the
  backward-compatible equivalent for inactive connections
  (`connection.h:344-366`, `396-418`). Suspend/resume calls balance
  (`connection.h:369-393`).
- `xpc_connection_send_message` is asynchronous (`connection.h:421-454`).
  `xpc_connection_send_message_with_reply` takes a reply handler and delivers
  the reply or an error dictionary on the connection's queue
  (`connection.h:509-543`); the `_sync` variant blocks and retains the returned
  object (`connection.h:548-582`).
- Cancel delivers the final `XPC_ERROR_CONNECTION_INVALID` event and invalidates
  outstanding replies (`connection.h:586-612`). `xpc_connection_get_name`
  returns the peer's Mach service name (`connection.h:615-628`).
- `xpc_dictionary_create_reply(original)` creates at most one reply for a
  received message (`xpc.h:1987`), and
  `xpc_dictionary_get_remote_connection` retrieves the connection that should
  receive it (`xpc.h:2105`).
- `xpc_main(handler)` never returns; it owns listeners created in the handler
  (`xpc.h:2698`). The D4 slice does not provide it.

## D4 scope

Implemented in the first slice: null, bool, int64, uint64, double, string,
data, array, dictionary and connection objects; error objects limited to the
two connection errors; type markers; retain/release/get_type/equal/hash/copy/
copy_description; dictionary and array accessors; connection create (named
service and listener), set_event_handler, set_target_queue (recorded only),
activate/resume/cancel, send, send-with-reply, send-with-reply-sync, reply
dictionary creation and remote-connection retrieval; listener check-in through
the same bootstrap namespace used by D2/D3.

Deferred: date, UUID, file descriptor and shared-memory objects; endpoints;
peer code-signing requirements; rich errors; transactions; activity scheduling;
`xpc_main`; dispatch-queue execution ordering (handlers run on Adapter's
connection dispatcher thread, not a guest GCD queue); Apple's private wire
format. Connections between Adapter workers use an adapter-defined protocol over
the existing loopback broker, in the same spirit as the D2 Mach broker. Guest
code that talks to a real macOS peer, or a real macOS process that talks to
Adapter, is out of scope until a compatible wire format is established.
