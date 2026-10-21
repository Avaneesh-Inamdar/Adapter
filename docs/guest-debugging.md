# Debugging Intel guests on Windows

`tools/guest_debug.py` launches the actual `adapter-run --worker --trace` process
under the installed LLVM LLDB. It records loaded Mach-O addresses before guest
constructors run, captures Windows exceptions and can stop or single-step at a
guest address. It does not debug the parent process and hope to follow its worker.

This is a Windows debugger bridge. Darwin exception ports, `task_for_pid`, guest
`debugserver`, complete Mach-O/DWARF unwinding and Xcode's debugger integration
remain separate work.

## Capture a fault

Build Adapter and the fixtures first. Keep the matching Windows PDB next to
`adapter-run.exe` so LLDB can resolve `adapter_debug_mapped`.

```powershell
cargo build --locked
python tests/build_fixtures.py
python tools/guest_debug.py --output target/debug-crash tests/fixtures/generated/crash
```

The output directory must be new. Existing directories are refused without
changing their contents. LLVM defaults to `C:\Program Files\LLVM\bin`;
`ADAPTER_LLVM` or `--lldb FILE` selects another installation. `--adapter FILE`
selects a build; the default is `target/debug/adapter-run.exe`.

| Artifact | Contents |
| --- | --- |
| `report.json` | Actual process state, stop reason, fault PC, image/segment and nearest symbol hint; register, instruction, stack and backtrace captures with their individual success flags |
| `images.jsonl` | One schema-1 record per mapped image: worker PID, path, header, slide, entry, segment VM/runtime ranges, final protection bits and bounded symbol hints |
| `guest.stdout`, `guest.stderr` | Raw guest output and runtime trace, redirected by LLDB |
| `lldb.log` | Batch debugger transcript and errors |
| `manifest.json` | Exact debugger command, worker arguments, adapter/guest SHA-256, start time, elapsed time and debugger exit code |
| `session.json` | Paths and options consumed by LLDB's embedded Python driver |

The debugger version is included in `report.json`. The runtime map includes at
most 16,384 nlist/export symbol hints per image, sets `symbols_truncated` when
that cap is reached and caps each name at 2,048 characters. A nearest label is
an address hint; it does not prove that the PC lies inside that function. A
stripped binary may have no useful label.

The tool returns zero when it successfully records an exited, stopped or crashed
process. Read the report's status and `guest_exit_code` to assess the guest.
`tool_error` and `timeout` return one. Argument errors return two. A failure to
create or launch a debugger target is never reported as a guest crash.

`crashed` requires an LLDB exception/signal stop. `fault_domain` is `guest` only
when the PC belongs to a recorded Mach-O segment; otherwise it is
`host_or_unmapped`. A nonzero normal exit, such as the `return37` probe, remains
`exited` with code 37.

## Breakpoints and instruction stepping

For an interactive terminal session, stop at the guest's LC_MAIN entry:

```powershell
python tools/guest_debug.py --interactive --timeout 3600 --output target/debug-interactive tests/fixtures/generated/crash
```

The script stops first at the runtime's mapped-image hook, reads its flushed map,
sets a breakpoint at the relocated guest entry and continues to that breakpoint.
The LLDB prompt uses the current terminal; no separate application is opened.
Guest stdout/stderr remain redirected to their files. Useful LLDB commands are:

```text
register read rip rsp rbp
disassemble --start-address $rip --count 12
thread step-inst
thread step-inst-over
breakpoint set -a 0xRUNTIME_ADDRESS
continue
process kill
quit
```

Use an actual runtime address from this session's map in `breakpoint set -a`.
Address-space randomization changes it between runs. Allow each interactive step
to finish before entering commands that inspect the stopped process.

To select a Mach-O VM address before the guest runs:

```powershell
python tools/guest_debug.py --break-vmaddr 0x1000002a4 --output target/debug-break tests/fixtures/generated/crash
python tools/guest_debug.py --step-instructions 1 --output target/debug-step tests/fixtures/generated/crash
```

The first command captures the breakpoint stop and ends the batch session. Add
`--interactive` to keep the prompt. `--image N` chooses the index in the map;
the default is the executable at index zero. The tool rejects a breakpoint outside
that image's executable segments. `--step-instructions 0..32` performs bounded
single-step execution after stopping at entry or `--break-vmaddr`, and records
each before/after PC. This executes guest instructions; it is not static analysis.

Pass guest arguments after the input file and `--`:

```powershell
python tools/guest_debug.py --output target/debug-jq tests/fixtures/generated/reference/jq-macos-amd64 -- -n "{ok:true}"
```

`--stdin FILE` supplies guest input. `--config FILE` supplies Adapter's loader
configuration. Stdin defaults to an empty file. The default whole-session
timeout is 30 seconds, including interactive time. On timeout the launcher kills
its own LLDB process tree and records `timeout`; a hanging guest does not become
a successful crash capture.

## Runtime integration and limits

The runtime writes maps only when `ADAPTER_DEBUG_MAP` is set. Without it, the
debug helper returns immediately. With it, map creation/writes are deliberate
debug operations and errors prevent guest entry. The helper flushes records after
final image protection and before calling the exported `adapter_debug_mapped`
hook. The CLI owns a fresh directory and supplies its map path to the worker.

Maps currently describe the initial prepared image graph. Later `dlopen` groups
are not recorded. Host adapter symbols come from the PE/PDB rather than the
guest symbol list. LLDB can display guest instructions in anonymously mapped
memory, but guest source-level stepping and reliable backtraces through all
optimized/frameless guest functions require additional symbol/unwind support.
Captured host/guest backtraces are diagnostic evidence, not proof that Darwin's
unwinder or exception propagation works.

The worker has the user's permissions. The tool preserves traces and stack
bytes in its output directory; choose and share that directory intentionally.

## Verification

On 2026-10-22, LLVM LLDB 23.1.3 on Windows x64 captured the unchanged Mach-O
`crash` fixture at its `ud2`: exception `0xc000001d`, guest VM address
`0x1000002a4`, nearest label `_main+4`. All five diagnostic commands succeeded.
A real instruction-step run stopped at `_main+1` after executing its one-byte
`push rbp` instruction. The tests also check exit 37, retained output/trace,
explicit VM breakpoints, invalid breakpoint/hook errors, refusal to overwrite
an existing directory and termination of the hanging worker after timeout.

The same nine tests run against the release build in the standard verifier. That
profile exposed a real defect: an empty hook body was folded by the release
linker into an unrelated function, so LLDB stopped before the map records were
written and reported a tool error. `adapter_debug_mapped` now keeps an
observable atomic generation counter, and all nine tests pass in debug and
release.

```powershell
python -m unittest discover -s tests/integration -p test_guest_debug.py -v
```

These are Windows-side debugger tests. A reference Mac is unnecessary for
observing the actual Windows fault; cross-platform semantic comparisons use
the separate differential harness.

Primary debugger references: [LLDB command-line options](https://lldb.llvm.org/man/lldb.html),
[Python API](https://lldb.llvm.org/use/python-reference.html),
[SBLaunchInfo](https://lldb.llvm.org/python_api/lldb.SBLaunchInfo.html) and
[LLDB command map](https://lldb.llvm.org/use/map.html).
