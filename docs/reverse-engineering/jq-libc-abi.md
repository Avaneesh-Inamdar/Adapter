# jq Intel macOS libc ABI evidence

Investigated 2026-10-21. Scope: static inspection of `tests/fixtures/generated/reference/jq-macos-amd64`, with primary Apple header/source corroboration. No macOS reference execution was available for this investigation. REA native analysis is unavailable on this Windows host as established by the parent investigation; LLVM disassembly was used instead. Finding IDs below are local report identifiers, not REA Evidence IDs.

## Artifact and method

- SHA-256: `E94B266E3C26690550006ABE63152B782280F4E14374ACCDF04CBDE844F00BC0`; size: 871,768 bytes.
- LLVM reports thin `MH_MAGIC_64`, x86-64, `MH_EXECUTE`, PIE, macOS minimum 14.0 / SDK 14.5.
- Preferred `__TEXT` VA is `0x100000000`, file offset zero; `__text` begins at VA `0x100000db0`, file offset `0xdb0`. Code locations below are unslid VAs; their file offsets are VA minus `0x100000000`.
- Tools actually run: `C:\Program Files\LLVM\bin\llvm-objdump.exe` and `clang.exe`, version 23.1.3, LLVM commit `0d261d1ca552c95a8f007e061c787ac7132fbcbc`.
- Commands: `llvm-objdump --macho --private-headers FILE`, `--syms FILE`, `--indirect-symbols FILE`, and `--disassemble --no-show-raw-insn FILE`.
- Layout experiment actually passed: feed mechanically reconstructed declarations matching the cited fields into `clang -cc1 -triple x86_64-apple-macosx14.0 -x c -fdump-record-layouts-complete -fsyntax-only -`. This validates compiler alignment/arithmetic for those declarations, not an Apple SDK build or runtime equivalence.

Apple sources were read at Libc commit [`71bbe350ab79eef58113991d817ccc6165061a64`](https://github.com/apple-oss-distributions/Libc/tree/71bbe350ab79eef58113991d817ccc6165061a64) and XNU commit [`f6217f891ac0bb64f3d375211650a4c1ff8ca1ea`](https://github.com/apple-oss-distributions/xnu/tree/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea). These are later than this binary's SDK; agreement with actual machine-code field accesses is called out separately.

## JQABI-1: RuneLocale data export and ctype

**Observed, high confidence:** the indirect symbol table places `__DefaultRuneLocale` at GOT slot `0x1000c8020`. At `0x10000102c`, jq loads the slot into RAX, then `0x100001033` reads a 32-bit value at `RAX + 0x3c + 4*RDI`. It tests mask `0x100` for alphabetic option characters. At `0x1000016d0` / `0x1000016d7`, the same table offset is tested with `0x4000` for whitespace. The slow branches call `___maskrune` at `0x100001481` and `0x100001807`; another whitespace test/slow call occurs after strptime at `0x10000d7bc` / `0x10000da1c`.

**Resulting requirement:** resolve Mach-O `__DefaultRuneLocale` to the structure itself. It is C `_DefaultRuneLocale` with the Mach-O name prefix, not a pointer variable. Provide a populated cached table at offset 60; patching only an `isalpha` function cannot fix these inline reads. C `__maskrune` is Mach-O `___maskrune` and takes signed 32-bit character plus Darwin 64-bit unsigned-long mask, returning masked bits as `int`.

LP64 layout from the [Apple RuneLocale declaration](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/include/runetype.h), corroborated by the Clang layout experiment:

| Field | Offset | Bytes |
| --- | ---: | ---: |
| `__magic` | 0 | 8 |
| `__encoding` | 8 | 32 |
| `__sgetrune` / `__sputrune` | 40 / 48 | 8 each |
| `__invalid_rune` | 56 | 4 |
| `__runetype[256]` | 60 (`0x3c`) | 1024 |
| `__maplower[256]` | 1084 (`0x43c`) | 1024 |
| `__mapupper[256]` | 2108 (`0x83c`) | 1024 |
| `__runetype_ext` / `__maplower_ext` / `__mapupper_ext` | 3136 / 3152 / 3168 | 16 each |
| `__variable` | 3184 | 8 |
| `__variable_len` / `__ncharclasses` | 3192 / 3196 | 4 each |
| `__charclasses` | 3200 | 8 |

Size 3208, alignment 8. `_RuneRange`: count at 0, pointer at 8, size16. `_RuneEntry`: min/max/map at 0/4/8, types pointer16, size24. `_RuneCharClass`: name[14] at0, mask16, size20/alignment4.

The [ctype header](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/include/_ctype.h) defines these classification bits:

| Class | Mask | Class | Mask |
| --- | --- | --- | --- |
| Alpha | `0x100` | Control | `0x200` |
| Digit | `0x400` | Graph | `0x800` |
| Lower | `0x1000` | Punctuation | `0x2000` |
| Space | `0x4000` | Upper | `0x8000` |
| Hex digit | `0x10000` | Blank | `0x20000` |
| Print | `0x40000` | Ideogram | `0x80000` |
| Special | `0x100000` | Phonogram | `0x200000` |

Width masks are `SW0=0x20000000`, `SW1=0x40000000`, `SW2=0x80000000`, `SW3=0xc0000000`, combined mask `0xe0000000`. These are Darwin flags, not CRT ctype bits. ASCII uses the default table directly; other characters call the locale-sensitive helper. Negative characters/EOF produce no extended classification in [runetype.c](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/locale/FreeBSD/runetype.c).

The [default table](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/locale/FreeBSD/table.c) uses magic `RuneMagA`, encoding `NONE`, invalid rune `0xfffd`, null conversion callbacks, and ASCII flags. Decimal/hex digit values occupy the low byte (`'A'`/`'a'` =10). Printable ASCII entries do not include `SW1`. Classification entries128..255 are implicitly zero; case-mapping entries there are identity. Tab includes control/space/blank; LF through CR include control/space; ordinary space includes space/blank/print.

## JQABI-2: Locale categories and masks

**Observed, high confidence:** startup call at `0x100000dde` invokes `_setlocale` with EDI=0 and RSI pointing to `""`. The return value is not checked before the next call. This is Darwin `setlocale(LC_ALL, "")`.

The [category header](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/include/locale.h) and [mask header](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/include/_locale_posix2008.h) establish:

| Category | Category value | Mask |
| --- | ---: | ---: |
| `LC_ALL` | 0 | 63 |
| `LC_COLLATE` | 1 | 1 |
| `LC_CTYPE` | 2 | 2 |
| `LC_MONETARY` | 3 | 8 |
| `LC_NUMERIC` | 4 | 16 |
| `LC_TIME` | 5 | 32 |
| `LC_MESSAGES` | 6 | 4 |

Masks are not `1 << category`. `LC_GLOBAL_LOCALE` is pointer -1; `LC_C_LOCALE` is NULL. [setlocale implementation](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/locale/FreeBSD/setlocale.c) treats NULL as query, empty string as environment selection, and rejects categories outside0..6. **Decision:** translate category numbers explicitly when delegating to Windows; reporting success for unsupported locale semantics would exceed this evidence.

## JQABI-3: FILE and stream exports

**Observed, high confidence:** indirect slots are `___stderrp=0x1000c8070`, `___stdinp=0x1000c8078`, `___stdoutp=0x1000c8080`. At `0x100000e3a`, jq loads the stdout slot, then loads `(%rax)` to obtain FILE*, and passes it to ferror/fclose. Stdin is similarly loaded at `0x1000372d3`; stderr at `0x100000e5c` feeds fwrite/fprintf paths. **Requirement:** these exports resolve to addresses of pointer variables. Returning FILE* directly at that symbol address introduces one missing level of indirection.

Layout from [Apple `_stdio.h`](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/include/_stdio.h), with compiler-layout corroboration:

| Field | Offset | Bytes |
| --- | ---: | ---: |
| `_p` | 0 | 8 |
| `_r` / `_w` | 8 / 12 | 4 each |
| `_flags` / `_file` | 16 / 18 | 2 each |
| `_bf._base` / `_bf._size` | 24 / 32 | 8 / 4 |
| `_lbfsize` | 40 | 4 |
| `_cookie` | 48 | 8 |
| `_close` / `_read` / `_seek` / `_write` | 56 / 64 / 72 / 80 | 8 each |
| `_ub._base` / `_ub._size` | 88 / 96 | 8 / 4 |
| `_extra` | 104 | 8 |
| `_ur` | 112 | 4 |
| `_ubuf[3]` / `_nbuf[1]` | 116 / 119 | 3 / 1 |
| `_lb._base` / `_lb._size` | 120 / 128 | 8 / 4 |
| `_blksize` | 136 | 4 |
| `_offset` | 144 | 8 |

Size152, alignment8. Flags: line-buffered1, unbuffered2, read4, write8, read/write16, EOF32, error64. `_file` is signed short; `fpos_t` is signed64. [Stream initialization](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/stdio/FreeBSD/findfp.c) initializes stdin flags4/fd0, stdout8/fd1, stderr10/fd2 and exports pointers into `__sF[3]`.

Reviewed jq sites call imported `getc`, `fputc`, `fileno`, `feof`, `ferror`, `fread`, and `fwrite`; direct FILE-field access remains unproven. **Inference:** keep guest and CRT objects separate. Zero `_r`, `_w`, `_lbfsize` initially selects header-defined unlocked slow paths; preserving this invariant needs tests.

## JQABI-4: stat64 and time structures

**Observed, high confidence:** `_fstat$INODE64` at `0x10002f11d` receives a buffer at RBP-`0xc0`. At `0x10002f127` jq reads a 16-bit value at buffer+4, masks `0xf000`, compares `0x4000`, and rejects directories. The non-directory path passes the descriptor to `_fdopen$DARWIN_EXTSN` at `0x10002f15d`.

[XNU stat declaration](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/sys/stat.h) and [underlying types](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/sys/_types.h), compiler-layout corroborated:

| stat64 field | Offset | Bytes |
| --- | ---: | ---: |
| dev / mode / nlink | 0 / 4 / 6 | 4 / 2 / 2 |
| inode | 8 | 8 |
| uid / gid / rdev | 16 / 20 / 24 | 4 each |
| atime / mtime / ctime / birthtime timespec | 32 / 48 / 64 / 80 | 16 each |
| size / blocks | 96 / 104 | 8 each |
| blksize / flags / generation / lspare | 112 / 116 / 120 / 124 | 4 each |
| qspare[2] | 128 | 16 |

Size144/alignment8. Each timespec contains signed64 seconds and nanoseconds. **Decision:** explicit translation is required; Windows `_stat` cannot be written into this buffer directly.

The [Darwin tm declaration](https://github.com/apple-oss-distributions/Libc/blob/71bbe350ab79eef58113991d817ccc6165061a64/include/_time.h), with [x86 types](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/i386/_types.h), has nine signed32 fields: sec0, min4, hour8, mday12, mon16, year20, wday24, yday28, isdst32; signed64 gmtoff40; zone pointer48. Size56/alignment8; Darwin `time_t` is signed64 on this target.

Actual jq evidence:

- `gmtime_r` at `0x10000e0ca` and `localtime_r` at `0x10000e1da` receive 64-bit time and guest tm buffers. Helper `0x10000ed90` reads tm fields: year+20 adds1900, then mon+16, mday+12, hour+8, min+4, sec+0, wday+24, yday+28. Those offsets corroborate the header prefix.
- `strptime` call `0x10000d79a` receives a 56-byte initialized tm. Instructions `0x10000d75e` / `0x10000d768` set tm_wday=8 and tm_yday=367 as sentinels before parsing. **Quirk:** adapters must preserve unspecified fields. Clearing the whole destination after parsing loses these sentinels.
- After strptime, `0x10000d7ab..0x10000d7d4` accepts NUL or a first unconsumed whitespace character. It does not validate the entire remaining suffix there. This is observed application control flow, not a general strptime rule.
- UTC strftime path sets `TZ=UTC` at `0x10000dc68`, calls strftime at `0x10000dc7b`, then restores or unsets TZ at `0x10000dc97` / `0x10000dca6`. Correct environment updates must be visible to time conversion.
- jq's mktime builtin calls `_timegm` at `0x10000df9c`; checks -2 as unsupported and -1 as invalid. That application check makes epoch -1 a special regression case.

## Remaining verification

Answered: the required cached RuneLocale offset, ctype flags, locale category/mask distinction, stream indirection, header LP64 layouts, and the listed jq static consumers. Partially answered: FILE field consumption (no exhaustive whole-program value-flow proof), extended Unicode locale behavior, complete date-format semantics. Unverified: macOS reference behavior, Windows adapter execution, exception/TLS interactions, and Apple SDK14.5 full-header compilation. No source/runtime files were edited and no compatibility test results are claimed by this report.
