# Mach IPC ABI evidence

Evidence for the D2 Mach IPC slice. The ABI comes from the pinned Xcode 27 XIP;
receive-side header rewriting comes from Apple's open-source XNU. This document
separates what was directly read from those sources from what this runtime
chooses to implement.

## Pinned header provenance

All paths are inside `Xcode.app` from `req files/Xcode_27.xip`
(SHA-256 `6a270c53a5a0c5e0ac78125342d44c3cfff2716ec390373cd5e30834d80a67c3`).
Extracted copies live under `target/artifacts/xcode27-sdk/mach/`; each has a
provenance JSON recording the archive member and source hash.

| Header | SDK copy used | SHA-256 | Size |
| --- | --- | --- | --- |
| `mach/message.h` | MacOSX | `86e9e87ba2a736236613f2c0a8563dbacaeec1a761654b12812eaf8599b55e67` | 37,637 |
| `mach/port.h` | AppleTVSimulator | `959cc098bb307ccc44a2193a9ff7b8ec8bbcedc335933512cdbe13d3c9088185` | 29,490 |
| `mach/mach_types.h` | XRSimulator | `2a5241cad8a10fccd42b0c9ae58f0d4af698bdc279ed80d7d427969157f819a7` | 11,602 |
| `mach/mach_port.h` | WatchSimulator | `c088bfd374e15020d6c0eadc43031d06a8fb05bfa5d1b1828d65d28618717a0f` | 46,568 |
| `mach/bootstrap.h` | MacOSX | `e9b68cb17a80344627edd64f57514f5c76de42d79d5ba789622ac2a976e21b93` | 3,356 |
| `mach/mach_init.h` | MacOSX | `2949d3efe703ff1d0111d1158e2af328d3828c5403441467c5199506576aa3fe` | 3,841 |
| `mach/mach.h` | MacOSX | `3380446992e918d0a7b8a3aaceef66094e8f4b47c01b42c4362720ac2fda8b4e` | 7,269 |
| `mach/kern_return.h` | MacOSX | `2d4b89f7cf91cf7bffafe457cdba8884c3f2e5596a9ef7d5e4a9ea4b239558d8` | 10,102 |
| `sys/_types/_mach_port_t.h` | AppleTVSimulator | `8f178c4c396766f74c2cf8240af9ca420aef38b67bd0d6b7b1ec84c2c48dfc91` | 2,098 |

The XIP stores SDK copies as hardlink groups: only the first CPIO member in
archive order carries data. The `port.h`, `mach_types.h` and `mach_port.h`
copies that first appear in other platform SDKs are zero-filled placeholders;
they were retained under `target/artifacts/xcode27-sdk/placeholders/` and are
**not** evidence. The carriers above were selected from `inventory.jsonl`
archive order, not by platform preference.

`mach_msg_header_t` is 24 bytes in the x86-64 user ABI: six 32-bit fields at
offsets 0, 4, 8, 12, 16 and 20 (`message.h:435-442`). A Clang static assertion
compiled for `x86_64-apple-macos10.13` confirms the size. The `kernel_user`
assertion macro only enforces the user-side size on non-arm64 targets
(`port.h:96-116`). `mach_msg_body_t` is 4 bytes; port descriptors are
`#pragma pack(4)` and 12 bytes in this ABI.

## Constants read from the pinned headers

Message bits (`message.h:138-179`, `240-249`):

- remote mask `0x1f`, local mask `0x1f00`, voucher mask `0x1f0000`
- complex `0x80000000`
- `MOVE_RECEIVE` 16, `MOVE_SEND` 17, `MOVE_SEND_ONCE` 18, `COPY_SEND` 19,
  `MAKE_SEND` 20, `MAKE_SEND_ONCE` 21
- normalized right types: `PORT_NONE` 0, `PORT_SEND` = `MOVE_SEND` 17,
  `PORT_SEND_ONCE` = `MOVE_SEND_ONCE` 18 (`message.h:669-674`)

Message options and results (`message.h:707-737`, `810-903`):

- `MACH_SEND_MSG` 0x1, `MACH_RCV_MSG` 0x2, `MACH_RCV_LARGE` 0x4
- `MACH_SEND_TIMEOUT` 0x10, `MACH_RCV_TIMEOUT` 0x100
- success 0, `MACH_SEND_INVALID_DATA` 0x10000002,
  `MACH_SEND_INVALID_DEST` 0x10000003, `MACH_SEND_INVALID_RIGHT` 0x1000000a,
  `MACH_SEND_INVALID_TYPE` 0x1000000f
- `MACH_RCV_INVALID_NAME` 0x10004002, `MACH_RCV_TIMED_OUT` 0x10004003,
  `MACH_RCV_TOO_LARGE` 0x10004004, `MACH_RCV_PORT_DIED` 0x10004009

Port rights (`port.h:205-209`): send 0, receive 1, send-once 2, port set 3,
dead name 4. `MACH_PORT_NULL` is 0 and `MACH_PORT_DEAD` is `0xffffffff`.
`mach_task_self_` is a global variable, not a function; `bootstrap_port` is a
global send right (`mach_init.h`, `bootstrap.h:70-71`).

Bootstrap (`bootstrap.h`): `BOOTSTRAP_MAX_NAME_LEN` 128, success 0,
`BOOTSTRAP_NAME_IN_USE` 1101, `BOOTSTRAP_UNKNOWN_SERVICE` 1102,
`BOOTSTRAP_SERVICE_ACTIVE` 1103. `bootstrap_check_in` and `bootstrap_look_up`
both return a port name through their final `mach_port_t *` argument.

## Received-header semantics

The send header is not the layout the receiver sees. XNU's
`ipc_kmsg_copyout_header` (`osfmk/ipc/ipc_kmsg.c`, commit
`f6217f891ac0bb64f3d375211650a4c1ff8ca1ea`, lines 3636-3640) does:

```c
msg->msgh_bits = MACH_MSGH_BITS_SET(reply_type, dest_type, voucher_type, mbits);
msg->msgh_local_port = CAST_MACH_NAME_TO_PORT(dest_name);
msg->msgh_remote_port = CAST_MACH_NAME_TO_PORT(reply_name);
```

That is, on receive:

- `msgh_local_port` is the destination port name in the receiver's space.
- `msgh_remote_port` is the reply port name in the receiver's space. For a
  send-once reply right this name is consumed by the reply send.
- the port-type fields swap: the remote bits describe the reply right, the
  local bits describe the destination right.

On the send side, `ipc_object_copyin_type` (`osfmk/ipc/ipc_object.c`,
lines 488-508) normalizes dispositions: `MOVE_SEND`, `MAKE_SEND` and
`COPY_SEND` all become `PORT_SEND` 17; `MOVE_SEND_ONCE` and `MAKE_SEND_ONCE`
become `PORT_SEND_ONCE` 18. A typical request sent with
`MACH_MSGH_BITS(COPY_SEND, MAKE_SEND_ONCE)` therefore arrives with bits
`(17 << 8) | 18` (`0x1112`), `msgh_local_port` equal to the service receive
name, and `msgh_remote_port` naming a send-once reply right.

The D2 fixtures assert this swap directly. It is the strongest check that the
adapter is not merely copying the sender's header into the receiver's buffer.

## Bootstrap check-in contract

The pinned `bootstrap.h` has no prose. The macOS 13.6 manual page says
`bootstrap_check_in` returns the receive right for a service that launchd has
created on the job's behalf, and returns `BOOTSTRAP_SERVICE_ACTIVE` when the
job has already checked the service in. It does not describe a caller-supplied
receive right. D2 has no launchd yet, so `bootstrap_check_in` creates the
receive right itself and registers it with the broker; duplicate check-in
returns `BOOTSTRAP_SERVICE_ACTIVE` 1103. This is a deliberate D2 behavior, not
a claim that launchd-managed check-in is complete.

`bootstrap_look_up` for an unknown name returns `BOOTSTRAP_UNKNOWN_SERVICE`
1102. Launchd will later own the namespace and pre-create service ports; D3
should revisit the check-in split.

## What D2 deliberately does not implement

- complex messages (descriptors, out-of-line memory, port descriptor arrays,
  vouchers) and message trailers
- port sets, dead-name notifications, send-once notifications, no-senders
  notifications
- `mach_port_extract_right`, `mach_port_get_refs`, `mach_port_peek`,
  `mach_port_names`, `mach_port_type`
- strict-reply vouchers, priority/QoS propagation, audit tokens
- `bootstrap_register`, `bootstrap_create_service`, `bootstrap_subset`
- launchd-managed port demand creation (D3)

The broker is an implementation of the routing that the kernel would perform;
it is not a Mach kernel. Rights are bookkeeping in each process plus a liveness
check at the broker. There is no memory protection between guests.
