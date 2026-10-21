# ABI contracts

The guest ABI is a shared interface between subsystems. A host implementation
with the same function name is not sufficient. The contracts in
[contracts.json](contracts.json) define the currently implemented slices;
[objc-abi.md](objc-abi.md) specifies the Objective-C slice in detail (contract v2
adds relative method lists and startup categories). The threads contract is at
version 2: it now declares the public `pthread_mutex_t`, `pthread_cond_t`,
`pthread_attr_t` and `pthread_once_t` sizes that the same-binary threading
suite exercises, and its unsupported list is narrowed to what remains
unverified.

Before extending a subsystem:

1. Pin the guest header/source revision and a matching binary observation.
2. Specify sizes, offsets, alignment, calling convention and return ABI.
3. Specify ownership, callbacks, reentrancy, errors and teardown.
4. List unsupported cases and the point at which they are rejected.
5. Add an actual Mach-O consumer and the reference probes where available.
6. Update the contract and run `python tools/check_contracts.py`.

Parallel contributors own separate modules. A change to a shared layout or
callback signature must update the owning contract and affected consumers in
the same change. New library symbols do not automatically imply complete API
semantics. Planned runtimes/services must add a contract before exposing their
first adapter; no empty placeholder is treated as an implemented contract.

The validator checks required fields, field bounds, cited repository paths and
the Objective-C registry against its declared symbols. It does not prove
behavioral equivalence. Runtime and differential tests provide that evidence.
