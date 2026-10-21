# Contributing

Use a Windows x64 development host with the prerequisites in the README. Keep
changes scoped to a measured target/API requirement, and preserve existing
loader, lifecycle and ABI probes while adding support.

## Development loop

```powershell
cargo fmt --all
python tools/verify.py
```

The verifier stops at the first failed stage and saves logs plus
`docs/test-results/latest.json`. It checks formatting, Rust Clippy with warnings
as errors, Rust tests, clang-format checks, native bridge compilation with C warnings as errors,
fixture generation, archive tests, and debug/release integration suites. The jq
artifact is fetched once from its pinned public release and verified by hash.
No reference Mac is needed for these tests.

## Compatibility changes

Explain the concrete consumer, binary version/hash and observed failure. Link
primary ABI/header evidence and distinguish static inference from execution.
Guest interfaces use Darwin x86-64 SysV; host calls use Windows conventions.
Translate data structures and varargs explicitly. Never pass Darwin FILE,
stat, tm, va_list or pthread layouts directly to the Windows CRT.

Add a meaningful Mach-O consumer or real-binary regression for an ABI change,
including errors/bounds where relevant. Unsupported inputs must fail clearly.
Report the tested scope in `docs/compatibility.md`, and update the README map
and roadmap when a milestone changes. Run `python tools/update_progress.py` to
regenerate the README counts, API bars and missing-import lists after adapter changes.
Passing a small consumer is not proof of
complete framework or macOS compatibility. Reference-Mac claims need actual
same-artifact comparison evidence.

## Source and test artifacts

Commit source, fixture builders and compact evidence reports. Keep downloaded
binaries, proprietary archives, SDK contents, extraction outputs and generated
Mach-O files out of Git. Do not modify supplied files under `req files/`.
Use the existing SHA-256/provenance workflow for external compatibility targets.

Pull requests should describe the behavior change, relevant validation and
remaining limitations. Repository-wide refactors and new dependencies should
have a concrete need established by the target or test evidence.
