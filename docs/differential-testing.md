# Same-binary Darwin/Windows differential testing

The harness executes one Intel Mach-O on native macOS and then the **same downloaded bytes** through `adapter-run.exe` on Windows. Two suites use it: `adapter-libc-v1` (4,096 deterministic cases across eight libc families) and `adapter-objc-v1` (4,096 cases across eight Objective-C families). A generator/build alone does not execute the guest and does not establish reference behavior.

## Commands

On macOS with the existing Clang/system linker:

```sh
python3 tools/differential.py build --suite adapter-libc-v1 --output reference-bundle --count 4096
python3 tools/differential.py capture --bundle reference-bundle --output reference-bundle/reference.json
```

Build uses `clang -target x86_64-apple-macos10.13 -std=c11 -O1 -fno-builtin -fno-stack-protector -fno-asynchronous-unwind-tables -Wl,-no_uuid`. The C consumer declares its ABI without SDK headers; linking uses the Mac host's system library/toolchain. It never downloads an SDK. Build records compiler version/binary SHA-256, source SHA-256, command and host, and refuses an existing program/manifest. `--count` may be a balanced multiple of 8 in 8..65536; 4,096 is the reference milestone. The versioned suite identifier defaults to `adapter-libc-v1`; `--suite adapter-objc-v1` builds the Objective-C consumer described below, which adds `-fno-objc-arc -lobjc` so clang links the host's libobjc.

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

`.github/workflows/mac-reference.yml` runs only on `workflow_dispatch`, at the selected workflow commit. It uses standard `macos-15-intel`, read-only repository contents permission, pinned checkout/upload actions and no secret, paid/custom runner or package installation. It builds `reference-bundle`, explicitly captures `reference-bundle/reference.json`, then uploads the entire bundle as `mac-reference-<suite>-<commit>` for 14 days. A matrix job covers both suites, so one dispatch produces the libc and Objective-C bundles independently; either can fail without hiding the other. Failed captures remain invalid; an uploaded artifact alone does not mean the workflow passed.

GitHub's [official runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) lists Intel `macos-15-intel` among standard runners free for public repositories. This is a hosted macOS **VM on Apple's infrastructure**, not a physical Mac measurement. The workflow does not claim a physical reference machine. Checkout v7 was resolved through GitHub's primary API to `3d3c42e5aac5ba805825da76410c181273ba90b1`; upload-artifact v7.0.2 was verified as `cf430e030ddbb5b0abf93d22962f4752f3646cd9` on 2026-10-22.

Offline verifier tests: `python -m unittest discover -s tests/unit -p test_differential.py`. Synthetic unit-test captures exercise evidence validation only and are never published as macOS compatibility results, and they cover both suites' record contracts.

## Objective-C suite (`adapter-objc-v1`)

`tests/differential/objc_cases.m` declares its runtime and C entry points without SDK headers and is compiled by the same host clang. It defines a compiler-emitted root class, a subclass, a category carrying `+load` and one category-only class method, then emits one record per index. The eight families exist because they separate failures: a selector-set bug shows up only in `selector`, a dispatch bug in `dispatch`/`super`, a category-ordering bug in `category`/`loadorder`, and a metadata bug in `classinfo`.

| Family | Probe | Result vector |
| --- | --- | --- |
| selector | `sel_registerName` on a stack-built name, compared with `sel_getUid` | equality twice, name round-trip, name length |
| dispatch | `-add:with:` with seeded arguments on a fresh instance | seeded arithmetic result |
| super | subclass method calling `[super compute:]` | inherited `+1` result |
| category | class method the category replaces | replacement result |
| nil | `nil` receivers with integer returns | two zero results |
| classinfo | `class_getName` FNV-1a of class and superclass, `class_getInstanceSize`, `class_isMetaClass` | two hashes, size, meta flag |
| object | `class_createInstance`/`object_getClass`/`class_getName`/`object_dispose` | identity, name length |
| loadorder | `+load` order encoded digit-per-call | order code, call count |

Deliberate exclusions: the suite never probes runtime-owned retain counts, `objc_retain` on a root class without `-retain`, ARC, weak tables, exceptions, tagged or packed isa, or aggregate message returns, because those are documented divergences or unsupported rather than claims this slice makes. A first dispatch proved the point: `objc_retain` on a free-standing compiler root class aborts on Darwin (`unrecognized selector -[Probe retain]`) while Adapter counts runtime-owned references, so the probe now tests object identity instead.

## Recorded run (2026-10-22)

| Item | Recorded value |
| --- | --- |
| Native reference | GitHub Actions run `39667131995`, standard `macos-15-intel` hosted VM, workflow commit `6a03795` |
| Reference host | `Darwin 24.6.0` (`xnu-11417.140.69.711.44~1`), `x86_64`, Python 3.14.7 |
| Reference compiler | `/usr/bin/clang`, Apple clang 17.0.0 (`clang-1700.0.13.5`), SHA-256 `7f30f076d0e9c38f772a76449fca9da8cf97f6a3d43b94c90a00e4f9ce7ad39e` |
| Consumer program | `libc-differential`, 13,136 bytes, thin x86-64 executable, SHA-256 `2cd055d52ae1ae2dca3979a81bb20fab4e58b9e110ea6358aa9219ee6eece2bc` |
| Consumer source | `libc_cases.c`, SHA-256 `a9ebaa51834ecd02f5f390e24f96dcd6aad2131c2e1df478fe5a352a487b307c` |
| Cases | 4,096 (eight families, 512 each) |
| Windows candidates | `target/debug/adapter-run.exe` SHA-256 `0c7129e3db97079f756c79d18f412d395226b67080a048efe6f28aace8185102`; `target/release/adapter-run.exe` SHA-256 `a1f1c4bd36acd4ea32ac635a9c39c340683934ab0b627eea7caa15dad9b9fa40`; Windows 10.0.26300 x64, Python 3.12.10 |
| Result | `equal` in both profiles: 4,096 matched, 0 mismatched, 512/512 per family |

Retained evidence from that run:

| File | Bytes | SHA-256 (stored bytes) |
| --- | ---: | --- |
| `docs/test-results/mac-reference-39667131995.json.gz` | 111,675 | `69f3fe3551ba1cf9b594cf95788d509d28ee926ff3a4d93406149111c1e9ade5` |
| `docs/test-results/differential-candidate-debug.json.gz` | 112,684 | `c2197ea80ae255e29bea3f3d481551522215863f381efe6c2728bc537ebf659d` |
| `docs/test-results/differential-candidate-release.json.gz` | 112,686 | `7af45424d78fa909adf45472786cef40801879e66ed4baa443f82bb541a2bd34` |
| `docs/test-results/differential-debug.json` | 1,123 | `c81771326ba1d87d8c28404c8465fac9c795c370b8328eec6f79b13297428efd` |
| `docs/test-results/differential-release.json` | 1,117 | `5130fedc56b8c50bdaefcc7aa9785bdfb564ad699879c47bce76160e3d4a12ca` |

The reference gzip decodes to the 1,037,734-byte native capture
`reference.json` (SHA-256 `7dce9327b53558d25492fbc229e7d072aad21c2b6eec82d5e8324c8f8a95d308`,
`mode: native`, `system: Darwin`). The candidate gzips decode to captures with
SHA-256 `dd42a234e5f7f48df96794eaffc6960ac9e3ac804d1a7b985aed1e98eb580e18`
(debug) and `5fdcf94538133e9edae26f54f7bec9331254face235f5203474d19f256a54abc`
(release); each capture holds the 4,096 parsed records plus raw stdout and the
adapter identity above. The committed compare reports are the harness output for
those pairs.

Commands used on Windows, from the repository root:

```powershell
python tools/differential.py capture --bundle target/mac-reference-39667131995/mac-reference-6a03795bae6a421597bda7aa1c35d10695b0d0f0 --output target/mac-reference-39667131995/adapter-release.json --adapter target/release/adapter-run.exe
python tools/differential.py compare --reference target/mac-reference-39667131995/mac-reference-6a03795bae6a421597bda7aa1c35d10695b0d0f0/reference.json --candidate target/mac-reference-39667131995/adapter-release.json --output target/mac-reference-39667131995/comparison-release.json
```

An earlier comparison of this same bundle differed in 236 of 4,096 cases
(`strtol` 73, `timegm` 163). Both defects are fixed in
[bridge.c](../src/darwin/host/bridge.c) (`strtol` must set `EINVAL` for a base
outside 2..36 and for no conversion) and
[system.c](../src/darwin/host/system.c) (`timegm` rejects a normalized year
below 1900); the tables above are the post-fix runs.

REA corroborated the reference artifact without executing it. `rea
trace-dylib-resolution` on the bundle program produced Evidence
`ev_6e040dd78390e1477c06ae4ba10fc2203474dd8e0bdd5f405d2384e0d3014374`
([retained](reverse-engineering/differential-reference-macho.json)): parse
status `parsed`, one thin `x86_64` `execute` slice of 13,136 bytes, no code
signature, and exactly one `LC_LOAD_DYLIB` edge to `/usr/lib/libSystem.B.dylib`
(compatibility 1.0.0, current 1351.0.0). That confirms the compared program
imports only libSystem and matches the manifest hash; it does not characterize
the host's libSystem implementation. REA's native engines (Hopper, Ghidra, IDA)
are not installed on this Windows host, so `rea inspect-macho` reports
`capability_unavailable` and no decompilation-level claim is made here; the
metadata above comes from REA's artifact-graph provider.

## Recorded Objective-C run (2026-10-22)

| Item | Recorded value |
| --- | --- |
| Native reference | GitHub Actions run `39673099968`, standard `macos-15-intel` hosted VM, workflow commit `de9fbd5` |
| Reference host | `Darwin 24.6.0` (`xnu-11417.140.69.711.44~1`), `x86_64`, Python 3.14.7 |
| Reference compiler | `/usr/bin/clang`, Apple clang 17.0.0 (`clang-1700.0.13.5`), SHA-256 `7f30f076d0e9c38f772a76449fca9da8cf97f6a3d43b94c90a00e4f9ce7ad39e` |
| Consumer program | `objc-differential`, 14,824 bytes, thin x86-64 executable, SHA-256 `95ab88d498c5bbf706f50b3392eef44b47a3c4f5536b1de743b103679f0e4f60` |
| Consumer source | `objc_cases.m`, SHA-256 `c76716d3964cf54835ba558d001481d36947b788d1a2388957be9ed9cd12ed4a` |
| Cases | 4,096 (eight families, 512 each) |
| Windows candidates | `target/debug/adapter-run.exe` SHA-256 `679f2856d568c00e50dba4255b75329539653ab1baa4e9a9433473fc963d1f0d`; `target/release/adapter-run.exe` SHA-256 `51bc98eea423f46cee072bb4b3a57a1c0a63778b167a7e42735e5a2c6ec7e92a`; Windows 10.0.26300 x64, Python 3.12.10 |
| Result | `equal` in both profiles: 4,096 matched, 0 mismatched, 512/512 per family |

Retained evidence from that run:

| File | Bytes | SHA-256 (stored bytes) |
| --- | ---: | --- |
| `docs/test-results/mac-reference-objc-39673099968.json.gz` | 63,014 | `dce37a14323c0fed8f2fccbe0f8193753203295fde2604a63756b56212c1531b` |
| `docs/test-results/differential-objc-candidate-debug.json.gz` | 62,941 | `eeeaf3c143db5ea12e52692df3d080466fecff7fc607d5da22aa85fc92ec647c` |
| `docs/test-results/differential-objc-candidate-release.json.gz` | 62,940 | `1ef5970d4b66409dacae4eb8f4eb97e03026b0fcc3744be10c9ea81866993226` |
| `docs/test-results/differential-objc-debug.json` | 1,085 | `5fe6033372b8cf84e57e49c2bed8a320c272ed56e2532b3165dcb0ca6b6b216a` |
| `docs/test-results/differential-objc-release.json` | 1,079 | `76b77154de743fa0562221887dd6e8e9a08e3adee21608953d106d268cd0b06f` |

The reference gzip decodes to the 926,224-byte native capture `reference.json`
(SHA-256 `4b6ab0b3e031a63f04e44ccbb0e2b2101322cebfbcdbd7257830995944a50172`,
`mode: native`, `system: Darwin`). The candidate gzips decode to captures with
SHA-256 `239e3746e4e5aeb04ae26be2b07bd81ed25a2bfadefffa00d1c24f8fbfb6dce9`
(debug) and `a63615abc6ee23a9749636dcc1c5bc31512cf6edb10e69a540a334dde112390d`
(release). The gzip inputs are reproducible from the bundle manifest: program
`objc-differential` SHA-256 `95ab88d4…`, source `objc_cases.m` SHA-256
`c76716d3…`.

The run also corrected an assumption rather than confirming one: Adapter had
been changed to report `class_getInstanceSize` as at least one word, and the
Darwin capture answered `0` for the compiler-emitted root class, so the change
was reverted and allocation keeps its own minimum. Before the fix the `classinfo`
family mismatched 512/512; after it every family matched.

Limits of this evidence: the reference is a hosted macOS VM, not a physical Mac;
the Objective-C suite covers eight families and excludes the divergences listed
above; JSONL equality is exact but `classinfo` carries 32-bit hashes. Threading,
Mach/XPC/launchd, jq and ripgrep still have no reference capture.
