"""Generate API coverage from recorded imports and the source adapter registries.

Default operation needs only this checkout. --refresh-imports reads the local,
hash-identified Apple corpus and pinned jq artifact; it never executes them.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import html
import json
from pathlib import Path
import re
import struct

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/progress'
SYSTEM = {'/usr/lib/libSystem.B.dylib', '/usr/lib/libSystem.dylib'}
CF = {'/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation',
      '/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation'}
XPC = {'/usr/lib/system/libxpc.dylib'}
OBJC = {'/usr/lib/libobjc.A.dylib'}


def imports(path, expected, expected_dependencies=None):
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError('Artifact hash mismatch: ' + str(path))
    # Select the same Intel-preferred slice as the existing inspection report.
    if data[:4] in (b'\xca\xfe\xba\xbe', b'\xca\xfe\xba\xbf'):
        fat64 = data[:4] == b'\xca\xfe\xba\xbf'
        count = struct.unpack_from('>I', data, 4)[0]
        if count > 64:
            raise ValueError('Too many FAT slices')
        slices = []
        for i in range(count):
            at = 8 + i * (32 if fat64 else 20)
            cpu = struct.unpack_from('>I', data, at)[0]
            offset, size = struct.unpack_from('>QQ' if fat64 else '>II', data, at + 8)
            slices.append((cpu, offset, size))
        _, offset, size = next((s for s in slices if s[0] == 0x1000007), slices[0])
        if offset + size > len(data):
            raise ValueError('FAT slice outside file')
        data = data[offset:offset + size]
    if data[:4] != b'\xcf\xfa\xed\xfe':
        raise ValueError('Expected little-endian Mach-O64')
    ncmds, cmdsize = struct.unpack_from('<II', data, 16)
    if ncmds > 16384 or 32 + cmdsize > len(data):
        raise ValueError('Invalid load-command bounds')
    deps, symtab, at = [], None, 32
    for _ in range(ncmds):
        cmd, size = struct.unpack_from('<II', data, at)
        if size < 8 or at + size > 32 + cmdsize:
            raise ValueError('Invalid load command')
        if cmd in (0xc, 0x80000018, 0x8000001f, 0x80000023, 0x20):
            name = struct.unpack_from('<I', data, at + 8)[0]
            if name < 24 or name >= size:
                raise ValueError('Invalid library string')
            deps.append(data[at + name:at + size].split(b'\0', 1)[0].decode())
        if cmd == 2:
            symtab = struct.unpack_from('<IIII', data, at + 8)
        at += size
    if symtab is None:
        raise ValueError('No symbol table')
    if expected_dependencies is not None and deps != expected_dependencies:
        raise ValueError('Library order differs from recorded inspector: ' + str(path))
    symoff, count, stroff, strsize = symtab
    if count > 1000000 or symoff + count * 16 > len(data) or stroff + strsize > len(data):
        raise ValueError('Symbol table outside file')
    result = defaultdict(set)
    for i in range(count):
        string, kind, _, desc, _ = struct.unpack_from('<IBBHQ', data, symoff + i * 16)
        if kind & 0xee:  # N_STAB or a defined N_TYPE
            continue
        if string >= strsize:
            raise ValueError('Symbol string outside table')
        end = data.find(b'\0', stroff + string, stroff + strsize)
        if end == -1:
            raise ValueError('Unterminated symbol name')
        raw = data[stroff + string:end]
        symbol = raw.decode()
        if not symbol:
            continue
        ordinal = desc >> 8
        library = deps[ordinal - 1] if 1 <= ordinal <= len(deps) else f'<special ordinal {ordinal}>'
        result[library].add(symbol)
    return {k: sorted(v) for k, v in sorted(result.items())}


def refresh():
    records = json.loads((ROOT / 'docs/reverse-engineering/apple-inputs/targets.json').read_text())
    rows = []
    for item in records:
        if item['architecture'] != 'x86_64':
            continue
        names = imports(ROOT / 'target/artifacts' / item['source'], item['sha256'], item['dependencies'])
        # Independent cross-check against the previously recorded Rust inspector.
        if set().union(*map(set, names.values())) != set(item['imports']):
            raise ValueError('Import inventory differs from inspector: ' + item['source'])
        rows.append({k: item[k] for k in ['name', 'source', 'sha256', 'architecture']}
                    | {'imports': names})
    jq = ROOT / 'tests/fixtures/generated/reference/jq-macos-amd64'
    digest = 'e94b266e3c26690550006abe63152b782280f4e14374accdf04cbde844f00bc0'
    rows.append({'name': 'jq 1.8.2', 'source': str(jq.relative_to(ROOT)).replace('\\', '/'),
                 'sha256': digest, 'architecture': 'x86_64', 'imports': imports(jq, digest)})
    rg = ROOT / 'tests/fixtures/generated/reference/ripgrep-macos-amd64'
    rg_digest = '0c9a0066db0d26b640777db88045b0ccdd58509a746700e43e1c4ff8707a5ed0'
    rows.append({'name': 'ripgrep 15.2.0', 'source': str(rg.relative_to(ROOT)).replace('\\', '/'),
                 'sha256': rg_digest, 'architecture': 'x86_64', 'imports': imports(rg, rg_digest)})
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'imports.json').write_text(json.dumps(rows, indent=2) + '\n', encoding='utf-8', newline='\n')


def registry():
    system = set()
    for p in (ROOT / 'src/darwin').glob('*.rs'):
        if p.name == 'xpc.rs':
            continue
        text = p.read_text()
        for arm in re.finditer(
                r'((?:"[^"\n]+"\s*(?:\|\s*)?)+)=>\s*(?:\{[^{}]*?)?\w+\s+as\s+\*const', text):
            system.update(re.findall(r'"([^"\n]+)"', arm.group(1)))
        system.update(re.findall(r'if name == "([^"\n]+)"', text))
    for p in (ROOT / 'src/darwin/host').glob('*.c'):
        text = p.read_text()
        system.update(re.findall(r'LOOKUP\("([^"\n]+)"\s*,', text))
        system.update('_' + n for n in re.findall(r'^\s*LM\((\w+)\);', text, re.M))
    # The binder is a terminating diagnostic, not an implemented API.
    system.discard('dyld_stub_binder')
    cf = set(re.findall(r'"([^"\n]+)"\s*=>\s*\w+\s+as\s+\*const',
                        (ROOT / 'src/frameworks/corefoundation.rs').read_text()))
    xpc_text = (ROOT / 'src/darwin/xpc.rs').read_text()
    xpc = set()
    for arm in re.finditer(
            r'((?:"[^"\n]+"\s*(?:\|\s*)?)+)=>\s*(?:\{[^{}]*?)?\w+\s+as\s+\*const', xpc_text):
        xpc.update(re.findall(r'"([^"\n]+)"', arm.group(1)))
    xpc.update(re.findall(r'"(__xpc_[a-z0-9_]+)"\s*=>\s*addr\(', xpc_text))
    objc_text = (ROOT / 'src/objc/mod.rs').read_text()
    objc = set()
    for arm in re.finditer(
            r'((?:"[^"\n]+"\s*(?:\|\s*)?)+)=>\s*\w+\s+as\s+\*const', objc_text):
        objc.update(re.findall(r'"([^"\n]+)"', arm.group(1)))
    objc.update(re.findall(r'"([^"\n]+)"\s*=>\s*\w+\.as_ptr\(\)\s+as\s+u64', objc_text))
    return {'libSystem': system, 'CoreFoundation': cf, 'libxpc': xpc, 'Objective-C': objc}


def family(library):
    if library in SYSTEM:
        return 'libSystem'
    if library in CF:
        return 'CoreFoundation'
    if library in XPC:
        return 'libxpc'
    if library == '/usr/lib/libobjc.A.dylib':
        return 'Objective-C'
    match = re.search(r'/([^/]+)\.framework/', library)
    return match[1] if match else 'Other libraries'


def available(library, name, adapters):
    group = ('libSystem' if library in SYSTEM else 'CoreFoundation' if library in CF
             else 'libxpc' if library in XPC else 'Objective-C' if library in OBJC else None)
    return group is not None and name in adapters[group]


def generate():
    records = json.loads((OUT / 'imports.json').read_text())
    adapters = registry()
    known, groups, targets, missing_frequency = set(), defaultdict(set), [], Counter()
    for item in records:
        if item['architecture'] != 'x86_64':
            continue
        pairs = {(lib, name) for lib, names in item['imports'].items() for name in names
                 if name != 'dyld_stub_binder'}
        known.update(pairs)
        for pair in pairs:
            groups[family(pair[0])].add(pair)
            if not available(*pair, adapters):
                missing_frequency[pair] += 1
        resolved = sum(available(*p, adapters) for p in pairs)
        targets.append({'name': item['name'], 'source': item['source'], 'sha256': item['sha256'],
                        'available': resolved, 'total': len(pairs),
                        'percent': round(100 * resolved / len(pairs), 2) if pairs else None,
                        'missing': [{'library': lib, 'symbol': name} for lib, name in sorted(pairs)
                                    if not available(lib, name, adapters)]})
    def count(label, pairs):
        have = sum(available(*p, adapters) for p in pairs)
        return {'name': label, 'available': have, 'total': len(pairs),
                'percent': round(100 * have / len(pairs), 2) if pairs else None}
    rows = [count('All known Intel imports', known)]
    rows += [count(k, v) for k, v in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0]))]
    report = {'metric': 'Source-registered adapters / unique (install name, imported symbol) pairs',
              'scope': f'{len(targets)} inspected Intel images; direct imports only',
              'not_measured': ['semantic completeness', 'transitive dependencies', 'ARM64 execution',
                               'whole macOS coverage', 'test pass rate'],
              'adapter_symbols': {k: sorted(v) for k, v in adapters.items()},
              'libraries': rows, 'targets': targets,
              'missing_by_target_count': [{'library': lib, 'symbol': name, 'target_count': n}
                                        for (lib, name), n in sorted(missing_frequency.items(),
                                                                    key=lambda item: (-item[1], item[0]))]}
    (OUT / 'status.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8', newline='\n')
    chart_rows = [rows[0]]
    main = ['libSystem', 'CoreFoundation', 'Objective-C', 'Foundation', 'AppKit', 'CoreGraphics']
    chart_rows += [row for label in main for row in rows[1:] if row['name'] == label]
    rest = [row for row in rows[1:] if row['name'] not in main]
    total = sum(row['total'] for row in rest)
    have = sum(row['available'] for row in rest)
    chart_rows.append({'name': 'Other APIs', 'available': have, 'total': total,
                       'percent': round(100 * have / total, 2) if total else 0})
    chart(chart_rows, len(targets))
    details(report)
    update_readme(report)
    for row in rows:
        print(f"{row['name']}: {row['available']}/{row['total']} ({row['percent']}%)")


def chart(rows, target_count):
    width, height = 960, 126 + 44 * len(rows)
    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc">',
           '<title id="title">Known Intel API adapter coverage</title>',
           '<desc id="desc">Registered adapters divided by imported library and symbol pairs. Does not measure full software compatibility.</desc>',
           f'<rect width="{width}" height="{height}" rx="12" fill="#111827"/>',
           '<g font-family="sans-serif" fill="#f9fafb">',
           '<text x="28" y="36" font-size="21" font-weight="600">Known Intel API adapter coverage</text>',
           f'<text x="28" y="62" font-size="13" fill="#cbd5e1">{target_count} Intel images · direct imports · source registry count</text>']
    for i, row in enumerate(rows):
        y = 90 + 44 * i
        pct = row['percent'] or 0
        color = '#34d399' if pct == 100 else '#fbbf24' if pct else '#64748b'
        svg += [f'<text x="28" y="{y+17}" font-size="14">{html.escape(row["name"])}</text>',
                f'<rect x="270" y="{y}" width="410" height="24" rx="4" fill="#374151"/>',
                f'<rect x="270" y="{y}" width="{410*pct/100:.2f}" height="24" rx="4" fill="{color}"/>',
                f'<text x="704" y="{y+17}" font-size="14" font-family="monospace">{pct:.2f}%</text>',
                f'<text x="804" y="{y+17}" font-size="14" fill="#cbd5e1">{row["available"]} / {row["total"]}</text>']
    svg += [f'<text x="28" y="{height-18}" font-size="12" fill="#9ca3af">An adapter may implement only part of an API. Missing APIs are listed in status.json.</text>', '</g></svg>']
    (OUT / 'api-coverage.svg').write_text('\n'.join(svg) + '\n', encoding='utf-8', newline='\n')


def details(report):
    lines = ['# API coverage inventory', '',
             'Generated by `python tools/update_progress.py` from `imports.json` and the adapter registries.', '',
             f'The denominator is unique (library install name, imported symbol) pairs in {len(report["targets"])} Intel images.',
             'It covers direct imports from the recorded slices. It does not include transitive dependencies,',
             'unobserved APIs, implementation completeness or runtime test results. ARM64 images are excluded.', '',
             '## Per-program imports', '',
             '| Image | Adapters available | Missing | API coverage |', '| --- | ---: | ---: | ---: |']
    for row in report['targets']:
        lines.append(f"| {row['name']} | {row['available']} / {row['total']} | {row['total']-row['available']} | {row['percent']:.2f}% |")
    lines += ['', 'A 100% row means the direct import names have adapters, not that the program is fully supported.',
              'jq and ripgrep have recorded execution tests. The Apple applications have inspection evidence only.', '',
              '## Missing APIs used by the most targets', '',
              '| Imported symbol | Library | Intel images using it |', '| --- | --- | ---: |']
    for row in report['missing_by_target_count'][:25]:
        lines.append(f"| `{row['symbol']}` | {family(row['library'])} | {row['target_count']} |")
    lines += ['', 'Full missing lists, library names and target hashes: [status.json](status.json).', '',
              'Library ordinals come from nlist_64.n_desc, as specified in the',
              '[Apple Mach-O header](https://github.com/apple-oss-distributions/xnu/blob/main/EXTERNAL_HEADERS/mach-o/nlist.h).',
              'Only exact supported library install names are matched. Imported data symbols are included.',
              'The terminating dyld_stub_binder diagnostic is not counted as an API adapter.', '',
              'Refresh source coverage after adding adapters with `python tools/update_progress.py`.',
              'Use `--refresh-imports` only when updating the artifact inventory; this requires the local',
              'hash-matched files under `target/artifacts` and the pinned jq download. The refresh cross-checks',
              'Apple import names and library order against the existing Rust inspection reports. No target is executed.', '']
    lines += ['Registry extraction covers the current Rust symbol matches and C LOOKUP/LM registrations.',
              'If a new adapter library or registration style is added, update `registry()` and `available()`',
              'in the generator as part of that change. Counts indicate a registration exists, not complete API semantics.', '']
    (OUT / 'README.md').write_text('\n'.join(lines), encoding='utf-8', newline='\n')


def update_readme(report):
    path = ROOT / 'README.md'
    text = path.read_text(encoding='utf-8')
    start, end = '<!-- API-COVERAGE:START -->', '<!-- API-COVERAGE:END -->'
    total = report['libraries'][0]
    lines = [start, '', '[![API adapter coverage](docs/progress/api-coverage.svg)](docs/progress/README.md)', '',
             f"**{total['available']:,} / {total['total']:,} known Intel imports have adapters: {total['percent']:.2f}%.**", '',
             'This counts unique **(library install name, imported symbol)** pairs from',
             f"{sum(1 for t in report['targets'] if t['name'] not in ('jq 1.8.2', 'ripgrep 15.2.0'))} inspected Apple Intel images, jq 1.8.2 and ripgrep 15.2.0. It includes imported data.",
             'An adapter may implement only part of an API. The percentage measures adapter',
             'availability, not application compatibility or total macOS completion.',
             'ARM64 and transitive dependencies are outside this count.', '',
             '| Program | Direct imports with adapters | API coverage | Execution status |',
             '| --- | ---: | ---: | --- |']
    statuses = {'jq 1.8.2': '14 recorded tests passed', 'ripgrep 15.2.0': '11 recorded CLI tests passed',
                'LogTool': 'Inspected; not run',
                'Clipboard Viewer': 'Inspected; framework classes/AppKit missing'}
    for name, status in statuses.items():
        row = next(t for t in report['targets'] if t['name'] == name)
        lines.append(f"| {name} | {row['available']} / {row['total']} | {row['percent']:.2f}% | {status} |")
    lines += ['| Xcode 27 launcher | — | — | ARM64; no translator |', end]
    at = text.index(start)
    stop = text.index(end, at) + len(end)
    path.write_text(text[:at] + '\n'.join(lines) + text[stop:], encoding='utf-8', newline='\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--refresh-imports', action='store_true')
    args = parser.parse_args()
    if args.refresh_imports:
        refresh()
    generate()
