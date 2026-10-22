"""Render the dependency graph and work list from docs/phases.json."""
import argparse
import html
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def outputs():
    data = json.loads((ROOT / 'docs/phases.json').read_text())
    phases = data['phases']
    lines = ['# Delivery phases', '', data['scope'], '',
             'Status describes the listed slice. It is not a percentage of macOS or Xcode.', '',
             '[Phase graph](progress/phases.svg) · [ABI contracts](abi/README.md) · '
             '[Pinned upstream sources and reuse limits](upstream-sources.md) · '
             '[Per-item dispositions: host work versus Apple-input gates](item-gates.md)', '',
             '## Architecture decision', '']
    lines += [v for k, v in data['architecture_decision'].items() if isinstance(v, str)]
    for phase in phases:
        lines += ['', f"## {phase['id']}: {phase['name']}", '',
                  f"Status: **{phase['status']}**. Dependencies: "
                  + (', '.join(phase['depends_on']) or 'none') + '.', '',
                  '| # | Work item | Status | Remaining work / boundary |',
                  '| ---: | --- | --- | --- |']
        for item in data['items']:
            if item['phase'] == phase['id']:
                lines += [f"| {item['id']} | {item['name']} | {item['status']} | {item['work']} |"]
        lines += ['', '**Exit gate:** ' + phase['exit_gate']]
    lines += ['', '## Binary acceptance gates', '',
              'A gate passes only with the actual named guest program and retained execution '
              'evidence. Registered API counts and substitute host programs do not pass a gate.', '',
              '| Gate | Result | Acceptance |', '| --- | --- | --- |']
    for gate in data['milestones']:
        result = f"[PASS](../{gate['evidence']})" if gate['passed'] else 'NOT PASSED'
        lines += [f"| {gate['id']}: {gate['name']} | {result} | {gate['acceptance']} |"]
    lines += ['', 'Edit [phases.json](phases.json), then run `python tools/check_contracts.py` '
              'and `python tools/update_phases.py --write`. The default command detects stale output.', '']
    # Wide graph keeps full phase labels readable in the README.
    positions = {'P0': (25, 45), 'P1': (290, 45), 'P2': (555, 45),
                 'P3': (820, 45), 'P4': (290, 225), 'P5': (555, 225), 'P6': (820, 225)}
    svg = ['<svg xmlns="http://www.w3.org/2000/svg" width="1100" height="420" '
           'viewBox="0 0 1100 420" role="img" aria-labelledby="title desc">',
           '<title id="title">Adapter delivery phases</title>',
           '<desc id="desc">Seven phases with dependency arrows. P0 evidence is in progress; '
           'P1 runtimes and P2 services have implemented slices. UI, ARM64 and Xcode remain.</desc>',
           '<rect width="1100" height="420" rx="12" fill="#101824"/>',
           '<defs><marker id="arrow" markerWidth="7" markerHeight="7" refX="6" refY="3.5" '
           'orient="auto"><path d="M0 0L7 3.5L0 7" fill="none" stroke="#94a3b8"/></marker></defs>']
    edges = [('P0', 'P1'), ('P1', 'P2'), ('P2', 'P3'), ('P1', 'P4'),
             ('P2', 'P5'), ('P3', 'P5'), ('P4', 'P5'), ('P5', 'P6')]
    for start, end in edges:
        x, y = positions[start]
        ex, ey = positions[end]
        if y == ey:
            path = f'M{x + 240} {y + 50}L{ex - 6} {ey + 50}'
        elif x == ex:
            path = f'M{x + 120} {y + 105}L{ex + 120} {ey - 6}'
        else:
            path = f'M{x + 120} {y + 105}L{x + 120} {y + 145}L{ex + 120} {y + 145}L{ex + 120} {ey - 6}'
        svg += [f'<path d="{path}" fill="none" stroke="#94a3b8" stroke-width="2" marker-end="url(#arrow)"/>']
    labels = {'P0': ('Evidence and contracts', 'Mac probes / guest debugger'),
              'P1': ('Loaders and runtimes', 'ARC / unwind discovery'),
              'P2': ('System services', 'IPC / XPC slices'),
              'P3': ('Graphics and desktop', 'NSWindow gate pending'),
              'P4': ('ARM64 / arm64e', 'Backend decision recorded'),
              'P5': ('Xcode integration', 'Five binary gates pending'),
              'P6': ('Instruments and devices', 'Simulator deferred')}
    for phase in phases:
        key = phase['id']
        x, y = positions[key]
        color = '#38bdf8' if key == 'P0' else '#fbbf24' if key in ('P1', 'P2') else '#94a3b8'
        title, subtitle = labels[key]
        svg += [f'<rect x="{x}" y="{y}" width="240" height="105" rx="8" fill="#1e293b" stroke="{color}"/>',
                f'<g font-family="Segoe UI,Arial,sans-serif"><text x="{x + 14}" y="{y + 25}" fill="{color}" font-size="15">{key} · {html.escape(phase["status"])}</text>',
                f'<text x="{x + 14}" y="{y + 53}" fill="#f8fafc" font-size="18">{html.escape(title)}</text>',
                f'<text x="{x + 14}" y="{y + 80}" fill="#cbd5e1" font-size="14">{html.escape(subtitle)}</text></g>']
    svg += ['<g fill="#cbd5e1" font-family="Segoe UI,Arial,sans-serif" font-size="15">',
            '<text x="25" y="370">All 36 requested work items are assigned. Arrows show dependencies; phases contain bounded slices.</text>',
            '<text x="25" y="397">API coverage is measured separately. No overall macOS completion percentage is claimed.</text></g></svg>', '']
    return {'docs/phases.md': '\n'.join(lines), 'docs/progress/phases.svg': '\n'.join(svg)}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    for name, content in outputs().items():
        path = ROOT / name
        if args.write:
            path.write_text(content, encoding='utf-8', newline='\n')
        elif not path.is_file() or path.read_text(encoding='utf-8') != content:
            raise SystemExit(f'{name} is stale; run python tools/update_phases.py --write')
    print('Phase graph and work list are current.')
