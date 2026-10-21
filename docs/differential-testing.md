# Same-binary Darwin/Windows differential testing

The suite executes one Intel Mach-O on native macOS and then the **same downloaded bytes** through `adapter-run.exe` on Windows. It compares 4,096 deterministic JSONL cases across 8 libc families. A generator/build alone does not execute the guest and does not establish reference behavior.

## Commands

On macOS with the existing Clang/system linker:

```sh
python3 tools/differential.py build --output reference-bundle --count 4096
python3 tools/differential.py capture --bundle reference-bundle --output reference-bundle/reference.json
```

Build uses `clang -target x86_64-apple-macos10.13 -std=c11 -O1 -fno-builtin -fno-stack-protector -fno-asynchronous-unwind-tables -Wl,-no_uuid`. The C consumer declares its ABI without SDK headers; linking uses the Mac host's system library/toolchain. It never downloads an SDK. Build records compiler version/binary SHA-256, source SHA-256, command and host, and refuses an existing program/manifest. `--count` may be a balanced multiple of 8 in 8..65536; 4,096 is the reference milestone. The versioned suite identifier is `adapter-libc-v1`.

Download the **whole** bundle, retaining its binary, source, manifest and native capture. On Windows, from the repository:

```powershell
python tools/differential.py capture --bundle reference-bundle --output candidate.json --adapter target/debug/adapter-run.exe
python tools/differential.py compare --reference reference-bundle/reference.json --candidate candidate.json --output differential-results.json
```

Capture is the explicit execution step. It verifies a thin x86-64 Mach-O executable and the manifest's program hash before launching. Windows mode also checks/hashes the x86-64 PE adapter. Default guest timeout is 60 seconds, configurable with `--timeout`; the adapter enforces this timeout and the Python process allows 5 seconds for wrapper cleanup. The child receives empty stdin and `LC_ALL=C`, `LANG=C`, `TZ=UTC0`. Raw stdout/stderr, exit status, host metadata, adapter identity and parsed records are retained. A timeout, malformed output, duplicate/missing/reordered IDs, wrong result shape or nonzero exit creates an invalid capture and exits 2.

Compare revalidates stdout rather than trusting the stored parsed array. It requires a successful **native Darwin** reference, matching program SHA-256/source identity/count, and a complete ordered case set on each side. Missing reference files fail; Windows output cannot serve as the Darwin reference. Values compare exactly, with strcmp/memcmp normalized in the consumer. Exit 0 means equal, 1 means semantic differences, 2 means invalid/incomplete evidence. The JSON report includes per-family totals and up to 100 mismatch examples while counting every difference. Hash/provenance fields attest the recorded execution inputs; they are not cryptographic attestation of a remote machine.

## Case contract

Each global index chooses family `index % 8` and a local xorshift32 seed `0x6d2b79f5 ^ (index * 0x9e3779b9)`. No wall clock, addresses, host random source or locale-dependent strings enter the records. 4,096 produces 512 cases per family, all processed in one bounded loop with small stack buffers.

| Family | Inputs | Result vector |
| --- | --- | --- |
| strlen | Random non-NUL bytes and a terminator | length |
| strcmp | Shared prefixes, equal/shorter/modified strings, high bytes | sign(-1/0/1) |
| memcmp | Arbitrary bytes, zero/full lengths, modified positions | sign(-1/0/1) |
| memmove | Same buffer, both overlap directions, same-address/zero-length cases | FNV-1a32 of all 512 bytes |
| strtol | Bases 2/8/10/16/36; sign/whitespace/prefix/no-conversion/overflow | signed Darwin long, end offset, errno after explicitly resetting it |
| snprintf | Width/precision, signed int/long, hex, exact-binary-fraction double f/g, strings, capacities 0..96 | return value, FNV-1a32 of initialized 96-byte destination (including guards beyond the bound) |
| gmtime_r | UTC epochs including negative, 0, -1, 32-bit boundaries and random values | success, sec/min/hour/mday/mon/year/wday/yday/isdst/gmtoff |
| timegm | Out-of-range seconds/minutes/hours/month/day; 1800..2100 years and isdst variations | epoch, normalized sec/min/hour/mday/mon/year/wday/yday/isdst/gmtoff |

Darwin LP64 `struct tm` is declared and statically checked as 56 bytes, with gmtoff at 40 and zone pointer at 48. Pointer addresses and zone abbreviations are excluded. JSON numeric output uses matching C printf argument types (`unsigned int` IDs and `long long` vector elements). `-fno-builtin` prevents the targeted calls from being folded into compiler answers. The fixture's printf and process startup remain infrastructure dependencies; invalid output is a failure, never a skipped case. Hash comparison has the usual 32-bit collision limit; this bounded suite is evidence for selected cases, not proof of all libc behavior.

## Manual hosted Mac reference workflow

`.github/workflows/mac-reference.yml` runs only on `workflow_dispatch`, at the selected workflow commit. It uses standard `macos-15-intel`, read-only repository contents permission, pinned checkout/upload actions and no secret, paid/custom runner or package installation. It builds `reference-bundle`, explicitly captures `reference-bundle/reference.json`, then uploads the entire bundle as `mac-reference-<commit>` for 14 days. Failed captures remain invalid; an uploaded artifact alone does not mean the workflow passed.

GitHub's [official runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) lists Intel `macos-15-intel` among standard runners free for public repositories. This is a hosted macOS **VM on Apple's infrastructure**, not a physical Mac measurement. The workflow does not claim a physical reference machine. Checkout v7 was resolved through GitHub's primary API to `3d3c42e5aac5ba805825da76410c181273ba90b1`; upload-artifact v7.0.2 was verified as `cf430e030ddbb5b0abf93d22962f4752f3646cd9` on 2026-10-22.

Offline verifier tests: `python -m unittest discover -s tests/unit -p test_differential.py`. Synthetic unit-test captures exercise evidence validation only and are never published as macOS compatibility results. Building/capturing the optional Objective-C consumer is deferred until the primary libc bundle has actual Darwin and Windows captures.
