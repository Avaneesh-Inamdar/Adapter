# Mach VM milestone — 2026-10-22

An unchanged Mach-O consumer that allocates, writes, protects and releases guest
memory now runs through Adapter with Darwin return codes. The guest address space
is the host address space, so an allocation is a host reservation the runtime
records and can name; nothing is emulated by a shadow map.

## Implementation

- `mach_vm_allocate` with `VM_FLAGS_ANYWHERE` and fixed placement. Sizes are
  rounded up to 4,096 bytes, a zero size is `KERN_INVALID_ARGUMENT`, more than
  256 MiB is `KERN_NO_SPACE` (the same ceiling the segment loader applies), and a
  fixed request over a live range returns `KERN_NO_SPACE` instead of silently
  displacing anything.
- `mach_vm_deallocate` releases only ranges this adapter created. A guest that
  passes an arbitrary address, a wrong size or a foreign task port receives
  `KERN_INVALID_ADDRESS`/`KERN_INVALID_ARGUMENT`; host heap memory is never
  freeable through this interface.
- `mach_vm_protect` maps `VM_PROT_READ`/`WRITE`/`EXECUTE` onto host page
  protections, rejects unknown bits and keeps writable-executable mappings
  rejected, matching the segment loader's rule. `set_maximum` is accepted and
  recorded as advisory.
- `vm_allocate`, `vm_deallocate` and `vm_protect` are the same interfaces without
  the second `mach_vm` layer, so a consumer using either spelling gets identical
  answers.
- Return codes are Darwin's: `KERN_SUCCESS` 0, `KERN_INVALID_ADDRESS` 1,
  `KERN_PROTECTION_FAILURE` 2, `KERN_NO_SPACE` 3, `KERN_INVALID_ARGUMENT` 4.

## Consumer and assertions

`tests/fixtures/mach_vm.c` is a cross-compiled Mach-O that declares the ABI it
uses and nothing else. It asserts:

- an anywhere allocation is non-zero and page-aligned, and 16 KiB written into it
  reads back byte for byte;
- a fixed allocation over the live range returns `KERN_NO_SPACE`;
- a read-only protection change and the return to read-write succeed and leave
  the contents intact;
- writable-executable protection, an out-of-range protect, a null protect, an
  unaligned deallocate, a foreign task port, a zero-size allocation, a 1 TiB
  allocation and an unknown flag each return the expected code;
- after release, a second release is `KERN_INVALID_ADDRESS`, and the same fixed
  address can be reserved again and written.

Expected output: `mach_vm:allocate, write, protect, fixed and errors passed`.

## Verification

The fixture runs in classic and chained form and the release profile matches the
debug profile. It is part of the `integration-debug` and `integration-release`
stages of `tools/verify.py`, so a regression fails the verifier rather than a
separate script.

## Boundaries

- No region queries (`mach_vm_region`/`mach_vm_region_recurse`), sharing between
  tasks, sub-range release, inheritance, ledger accounting or `mach_vm_copy`/
  `read_overwrite`/`write`. Those remain unimplemented symbols and fail loudly.
- `VM_FLAGS_OVERWRITE`, `VM_FLAGS_PURGABLE` and purgeable state are not modelled;
  any flag other than anywhere/fixed is rejected.
- Protection of code pages the loader mapped is not retargeted; the adapter's
  segment protections stand.
- No page-level reference Mac comparison exists for this interface: the probe
  asserts documented Darwin return codes and its own invariants, and the same
  binary would need a hosted run to compare answers field by field.

## Reproduction

```powershell
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime.RuntimeTests.test_mach_vm_allocation_protection_and_errors
python tools/verify.py
```
