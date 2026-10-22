"""Validate versioned ABI declarations and the complete phase inventory."""
import json
from pathlib import Path

from update_progress import registry

ROOT = Path(__file__).resolve().parents[1]


def validate():
    contracts = json.loads((ROOT / 'docs/abi/contracts.json').read_text())
    assert contracts['schema_version'] == 1
    seen = set()
    for contract in contracts['contracts']:
        for key in ('id', 'version', 'scope', 'calling_convention', 'ownership',
                    'callbacks', 'errors', 'lifecycle', 'unsupported', 'sources',
                    'implementation', 'tests', 'layouts'):
            assert contract.get(key), f"{contract.get('id')}: missing {key}"
        assert contract['id'] not in seen, 'duplicate contract'
        seen.add(contract['id'])
        assert isinstance(contract['version'], int) and contract['version'] > 0
        for name in ('implementation', 'tests'):
            for path in contract[name]:
                assert (ROOT / path).is_file(), f'missing {path}'
        for source in contract['sources']:
            assert source.startswith('https://') or (ROOT / source).is_file(), f'missing source {source}'
        for layout in contract['layouts']:
            size, align = layout['size'], layout['align']
            assert size > 0 and align > 0 and align & (align - 1) == 0
            assert size % align == 0, f"unaligned {layout['name']}"
            end = 0
            for field in sorted(layout['fields'], key=lambda f: f['offset']):
                assert field['offset'] >= end and field['size'] > 0
                end = field['offset'] + field['size']
                assert end <= size, f"out-of-bounds {layout['name']}.{field['name']}"
        if contract['id'] == 'objc':
            declared = contract['symbols']
            assert len(declared) == len(set(declared)), 'duplicate ObjC symbol'
            actual = registry()['Objective-C']
            assert set(declared) == actual, (
                f'ObjC contract drift: undeclared={actual - set(declared)}, '
                f'absent={set(declared) - actual}')
    assert seen == {'loader', 'libc', 'threads', 'mach', 'mach_vm', 'blocks-xpc', 'cf',
                   'objc'}
    phases = json.loads((ROOT / 'docs/phases.json').read_text())
    assert phases['schema_version'] == 1
    entries = phases['phases']
    by_id = {p['id']: p for p in entries}
    assert len(by_id) == len(entries) == 7
    assigned = [i for p in entries for i in p['items']]
    assert sorted(assigned) == list(range(1, 37)), 'phase assignment must cover 1..36 once'
    items = phases['items']
    assert sorted(i['id'] for i in items) == list(range(1, 37))
    for item in items:
        assert item['id'] in by_id[item['phase']]['items']
        assert item['status'] and item['work']
        assert (ROOT / 'docs' / item['source']).is_file()
    visited, active = set(), set()

    def visit(name):
        assert name in by_id and name not in active, 'invalid/cyclic dependency'
        if name in visited:
            return
        active.add(name)
        for dependency in by_id[name]['depends_on']:
            visit(dependency)
        active.remove(name)
        visited.add(name)

    for name in by_id:
        visit(name)
    milestones = phases['milestones']
    assert [m['id'] for m in milestones] == ['M0', 'M1', 'M2', 'M3', 'M4']
    passed = set()
    for milestone in milestones:
        if milestone['passed']:
            assert all(d in passed for d in milestone['depends_on'])
            assert milestone['evidence'] and (ROOT / milestone['evidence']).is_file()
            passed.add(milestone['id'])
    print(f"Validated {len(seen)} ABI contracts, 36 work items, 7 phases and 5 binary gates.")


if __name__ == '__main__':
    validate()
