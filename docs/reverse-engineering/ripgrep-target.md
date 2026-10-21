# Target evidence: ripgrep 15.2.0 macOS amd64

This note separates what was observed from what is inferred. It does not claim
full compatibility, and it does not replay decompiler output. No REA analysis
provider was connected for this pass; the available REA engines are limited to
PE input (or require macOS), so evidence here is static Mach-O inspection plus
Windows-side execution of the unmodified artifact.

## Observed

| ID | Observation | Source |
| --- | --- | --- |
| O1 | Thin little-endian x86-64 Mach-O executable, filetype 2, 4,387,000 bytes, SHA-256 `0c9a0066db0d26b640777db88045b0ccdd58509a746700e43e1c4ff8707a5ed0` | `adapter-inspect`, `tools/fetch_ripgrep.py` |
| O2 | Direct dependencies are `/usr/lib/libiconv.2.dylib` and `/usr/lib/libSystem.B.dylib`; libiconv has no referenced imports | `adapter-inspect`, `docs/progress/imports.json` |
| O3 | 132 undefined symbols: 131 API names plus `dyld_stub_binder`; the coverage generator resolves 131 / 131 API names to adapters | `tools/update_progress.py`, `docs/progress/status.json` |
| O4 | The unchanged binary executes searches through `adapter-run.exe`; 11 integration methods pass in debug and release | `tests/integration/test_ripgrep.py`, `docs/test-results/` |
| O5 | Imported surface includes dispatch semaphores, signals/sigaltstack, pthread attributes/mutexes, `fork`/`execvp`/`waitpid`/`posix_spawnp`, dyld image introspection, `mmap` with `MAP_FIXED`, and `___chkstk_darwin` | symbol table; [inventory](../progress/README.md) |
| O6 | `--pre "cmd /c type"` exits 2 with `preprocessor command could not start`; no child process is created | `test_preprocessor_spawn_fails_cleanly` |
| O7 | Recursive search respects a `.gitignore` only when a `.git` directory is present, skips hidden entries by default, and includes them with `--hidden` | `test_recursive_ignore_and_hidden` |
| O8 | JSON output is line-delimited and ends with a summary event; PCRE2 and UTF-8 patterns match; stdin is preferred over `.`; binary files are summarized unless `-a` is used | `test_json_stream`, `test_pcre2_and_unicode`, `test_stdin_is_preferred_over_cwd`, `test_binary_detection_and_text_mode` |

## Inferred

- I1: The tested search, listing, JSON and threading workflows do not require
  Darwin signal delivery, condition variables, or child-process creation. The
  adapters that record signals, implement recursive mutexes, and reject spawning
  are sufficient for these paths.
- I2: `ripgrep`'s CRLF preservation in matched lines comes from its own
  line-oriented output. Adapter stdio is opened in binary mode and does not add
  carriage returns.
- I3: `dispatch_semaphore_*` is used by the worker pool; the SRWLock-backed
  semaphore adapters satisfy the tested 40-file multithreaded search.
- I4: The preprocessor failure is a deliberate unsupported-process boundary, not
  an accidental crash: the guest receives ENOSYS/ECHILD and emits its own
  diagnostic.

These inferences are bounded by the tests above. They are not claims that every
ripgrep code path behaves like macOS.

## Unknown / not yet evidenced

- Behavior of signal-handler paths (for example a broken output pipe or an
  externally delivered signal). Handlers are stored but never invoked.
- Any workflow that needs `fork`, `execvp`, `waitpid` or `posix_spawnp`, which
  remains unsupported.
- Condition variables, errorcheck/adaptive mutexes, cancellation and robust
  mutex semantics.
- Locale, timezone, extended-attribute, ACL and full `stat` metadata fidelity on
  macOS-specific filesystems.
- The complete ripgrep option matrix, unusual encodings and pathological file
  layouts beyond the 2 MiB mmap and 40-file cases.
- Instruction-level coverage of paths not reached by the integration suite.
  No decompiler evidence was produced in this pass.

## Why this target

jq exercised JSON, files, modules, dates and printf-family varargs but did not
pressure descriptor flags, directory iteration, mmap, dispatch, pthread
attributes or process stubs. ripgrep imports all of those, so it exposes loader
and adapter gaps that a second jq-style target would miss. It is also a pinned,
published CLI binary, which keeps the evidence reproducible without rebuilding
the guest.

## Next evidence needed

1. Run the same tests and a few error/pipe cases on a reference Mac and compare
   stdout, stderr and exit status byte-for-byte.
2. Add a spawn-capable consumer only when the runtime can model child processes;
   until then keep the ENOSYS path explicit and tested.
3. If a native decompiler provider becomes available, inspect the preprocessor
   and platform-probe paths rather than re-deriving the tested search paths.
4. Add signal-delivery evidence with a dedicated Mach-O probe before claiming
   handler semantics.
