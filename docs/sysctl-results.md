# sysctl and libproc milestone — 2026-10-22

An unchanged Mach-O consumer now reads system facts through `sysctl`,
`sysctlbyname` and `sysctlnametomib` and describes its own process through a
small `libproc` surface. The implementation is
[src/darwin/sysctl.rs](../src/darwin/sysctl.rs).

## What the adapter does

- **Names and MIBs.** Seventy-odd names answer: `kern.*` (OS type, release,
  version, build, host name, boot time, limits), `hw.*` (processor counts,
  memory, page size, cache sizes, time base, CPU features), `machdep.cpu.*`
  (CPUID vendor, brand string, family, model, stepping, core and thread counts),
  `user.*` limits and `vm.loadavg`/`vm.swapusage`. Names that have a fixed number
  in xnu's `sys/sysctl.h` use it (`hw.ncpu` is `{6, 3}`); names Darwin allocates
  dynamically (`hw.logicalcpu`, `machdep.cpu.*`) get adapter-assigned second
  components starting at 1000. `tools/update_progress.py` fails if a fixed number
  disagrees with the pinned header.
- **Host facts.** Processor counts, cache sizes and line size come from
  `GetLogicalProcessorInformation`; memory from `GlobalMemoryStatusEx`; boot time
  from the tick count; nominal frequency from the registry; CPUID strings and
  feature bits from the processor.
- **Darwin answers.** Strings agree with `uname`: `Darwin`, release `24.6.0`,
  machine `x86_64`; the build (`24G84`) and product version (`15.6`) are fixed
  adapter-reported values, and the model is `AdapterHost1,1`.
- **sysctl(3) semantics.** A NULL buffer asks for the size; a short buffer
  receives a prefix and the call fails with `ENOMEM` and the full size; every
  name is read-only (`EPERM` on write); unknown names and MIBs are `ENOENT`;
  bad arguments are `EINVAL`. `sysctl({0, 3}, ...)` is the name-to-OID call libc
  itself uses.
- **libproc.** `proc_libversion`; `proc_name`, `proc_pidpath`, `proc_listpids`
  and `proc_pidinfo` (`PROC_PIDTBSDINFO` and `PROC_PIDTASKINFO`) describe the
  guest's own pid. Any other pid is `ESRCH`.

## Consumer

[sysctl.c](../tests/fixtures/sysctl.c), classic and chained, asserts: strings
equal `uname`'s; size query and short-buffer behavior; processor-count
consistency (`ncpu` = `logicalcpu` = `activecpu`, `physicalcpu` between 1 and
that); 64-bit and 32-bit value sizes; a plausible `struct timeval` boot time;
MIB, `sysctlnametomib` and `{0, 3}` name2oid agreement; the error codes above;
and the libproc structures for its own pid.

## Boundaries

- No `kern.proc`/`kern.procargs2` (`struct kinfo_proc`, argument vectors),
  `kern.clockrate`, `vm.vmmeter`, `net.*`, `debug.*`, disk statistics or
  performance-level (`hw.perflevel*`) names. 60 of the 135 catalogued names are
  absent; the list is `public_api.missing` in
  [progress/status.json](progress/status.json).
- Values are plausible rather than measured on a Mac: cache sizes and counts
  follow the Windows host, limits are common Darwin defaults, and no
  reference-Mac comparison exists.
- Nothing is writable, so `sysctl -w`-style programs fail with `EPERM`.
- libproc covers one process. Process listing, file descriptor, region and
  resource-usage flavors, `proc_pid_rusage` and dirty tracking are absent.
- `proc_pidpath` returns the adapter's spelling of the executable path.

## Reproduction

```powershell
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime -k sysctl
python tools/update_progress.py
python tools/verify.py
```
