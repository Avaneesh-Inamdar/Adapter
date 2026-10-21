"""Same-binary CLI differential: run one real program on Darwin and through Adapter.

The corpus is a fixed argv/stdin list. Each case records the exit status and the
SHA-256 plus length of stdout and stderr, so the comparison never depends on
addresses, paths or wall clock. A capture is evidence of one explicit execution;
a build or a preflight alone is not.
"""
import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = 1
SUITE = 'adapter-cli-v1'
BINARIES = {
    'jq': ROOT/'tests/fixtures/generated/reference/jq-macos-amd64',
    'rg': ROOT/'tests/fixtures/generated/reference/ripgrep-macos-amd64',
}
CHILD_ENV = {'LC_ALL': 'C', 'LANG': 'C', 'TZ': 'UTC0'}
SRC_PREFIX = 512


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def read_json(path):
    return json.loads(Path(path).read_text(encoding='utf-8'))


def write_json(path, value):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=False) + '\n', encoding='utf-8')


def corpus_errors(corpus):
    errors = []
    if not isinstance(corpus, dict):
        return ['corpus is not an object']
    if type(corpus.get('schema')) is not int or corpus.get('schema') != SCHEMA:
        errors.append('corpus schema mismatch')
    if corpus.get('suite') != SUITE:
        errors.append('corpus suite mismatch')
    cases = corpus.get('cases')
    if not isinstance(cases, list) or not cases:
        return errors + ['corpus has no cases']
    seen = set()
    for case in cases:
        if not isinstance(case, dict) or set(case) != {'id', 'binary', 'argv', 'stdin'}:
            errors.append('invalid case record')
            continue
        if not isinstance(case['id'], str) or not case['id'] or case['id'] in seen:
            errors.append(f"invalid or duplicate case id {case.get('id')!r}")
        seen.add(case['id'])
        if case['binary'] not in BINARIES:
            errors.append(f"unknown binary {case['binary']!r}")
        if not isinstance(case['argv'], list) or any(not isinstance(a, str) for a in case['argv']):
            errors.append(f"invalid argv in {case['id']}")
        if not isinstance(case['stdin'], str):
            errors.append(f"invalid stdin in {case['id']}")
    return errors


def host_metadata():
    return {'system': platform.system(), 'release': platform.release(), 'machine': platform.machine(),
            'python': platform.python_version()}


def run_case(case, binary, adapter, timeout):
    command = [str(binary), *case['argv']]
    if adapter:
        command = [str(adapter), '--timeout', str(timeout), str(binary), '--', *case['argv']]
    env = {**os.environ, **CHILD_ENV}
    try:
        process = subprocess.run(command, input=case['stdin'].encode('utf-8', 'surrogateescape'),
                                 capture_output=True, timeout=timeout, cwd=ROOT, env=env)
    except subprocess.TimeoutExpired as exc:
        return {'id': case['id'], 'status': 'timed-out', 'timeout_seconds': timeout,
                'stdout_len': len(exc.stdout or b''), 'stderr_len': len(exc.stderr or b'')}
    out, err = process.stdout, process.stderr
    return {'id': case['id'], 'status': 'captured', 'exit_code': process.returncode,
            'stdout_sha256': sha256(out), 'stdout_len': len(out),
            'stderr_sha256': sha256(err), 'stderr_len': len(err),
            'stdout_prefix': out[:SRC_PREFIX].decode('utf-8', 'backslashreplace'),
            'stderr_prefix': err[:SRC_PREFIX].decode('utf-8', 'backslashreplace')}


def capture(corpus_path, output, adapter=None, timeout=30):
    corpus = read_json(corpus_path)
    errors = corpus_errors(corpus)
    if errors:
        raise ValueError('; '.join(errors))
    if adapter:
        if platform.system() != 'Windows':
            raise ValueError('adapter capture requires Windows; use native capture on Darwin')
        adapter = Path(adapter).resolve()
        if not adapter.is_file():
            raise ValueError('adapter executable not found')
        adapter_info = {'path': str(adapter), 'sha256': sha256(adapter.read_bytes())}
    else:
        if platform.system() != 'Darwin':
            raise ValueError('native reference capture requires Darwin; Windows requires --adapter')
        adapter_info = None
    binaries = {}
    for name, path in BINARIES.items():
        if not path.is_file():
            raise ValueError(f'{name} reference binary is missing; run the fetch stage first')
        binaries[name] = {'file': path.name, 'bytes': path.stat().st_size, 'sha256': sha256(path.read_bytes())}
    records, failures = [], []
    for case in corpus['cases']:
        record = run_case(case, BINARIES[case['binary']], adapter_info and adapter, timeout)
        records.append(record)
        if record['status'] != 'captured':
            failures.append(record['id'])
    payload = {'schema': SCHEMA, 'suite': SUITE, 'mode': 'adapter' if adapter_info else 'native',
               'host': host_metadata(), 'adapter': adapter_info, 'binaries': binaries,
               'corpus': {'file': Path(corpus_path).name, 'sha256': sha256(Path(corpus_path).read_bytes())},
               'child_environment': CHILD_ENV, 'timeout_seconds': timeout,
               'status': 'captured' if not failures else 'invalid',
               'errors': [f'{name} timed out' for name in failures], 'records': records}
    write_json(output, payload)
    return payload


def capture_errors(capture_value, reference=False):
    errors = []
    if not isinstance(capture_value, dict):
        return ['capture is not an object']
    if type(capture_value.get('schema')) is not int or capture_value.get('schema') != SCHEMA \
            or capture_value.get('suite') != SUITE:
        errors.append('capture schema/suite mismatch')
    mode, host = capture_value.get('mode'), capture_value.get('host', {})
    system = host.get('system') if isinstance(host, dict) else None
    if mode == 'native':
        if system != 'Darwin' or capture_value.get('adapter') is not None:
            errors.append('native capture host is not Darwin')
    elif mode == 'adapter':
        adapter = capture_value.get('adapter')
        if system != 'Windows' or not isinstance(adapter, dict) or len(str(adapter.get('sha256', ''))) != 64:
            errors.append('adapter capture lacks Windows/adapter identity')
    else:
        errors.append('unknown capture execution mode')
    if reference and (system != 'Darwin' or mode != 'native'):
        errors.append('reference must be a native Darwin capture')
    if capture_value.get('child_environment') != CHILD_ENV:
        errors.append('capture environment does not match the C/UTC contract')
    records = capture_value.get('records')
    if not isinstance(records, list) or not records:
        return errors + ['capture has no records']
    for record in records:
        if not isinstance(record, dict) or record.get('status') != 'captured':
            errors.append('capture contains a case without a result')
            continue
        if type(record.get('exit_code')) is not int or type(record.get('stdout_len')) is not int \
                or type(record.get('stderr_len')) is not int:
            errors.append(f"invalid record {record.get('id')!r}")
    if capture_value.get('status') != 'captured' or capture_value.get('errors') != []:
        errors.append('capture did not validate successfully')
    return errors


def compare(reference_path, candidate_path, output):
    report = {'schema': SCHEMA, 'suite': SUITE, 'status': 'invalid', 'errors': [],
              'reference': str(reference_path), 'candidate': str(candidate_path),
              'case_count': 0, 'matched': 0, 'mismatched': 0, 'differences': []}
    values = []
    for label, path in (('reference', reference_path), ('candidate', candidate_path)):
        try:
            value = read_json(path)
            values.append(value)
            report['errors'].extend(f'{label}: {error}' for error in
                                    capture_errors(value, reference=(label == 'reference')))
        except (OSError, ValueError, TypeError) as exc:
            values.append(None)
            report['errors'].append(f'{label}: {exc}')
    if not report['errors']:
        reference, candidate = values
        if reference['corpus'] != candidate['corpus']:
            report['errors'].append('reference and candidate corpus differ')
        if reference['binaries'] != candidate['binaries']:
            report['errors'].append('reference and candidate binaries differ')
        if [r['id'] for r in reference['records']] != [r['id'] for r in candidate['records']]:
            report['errors'].append('reference and candidate case order differs')
    if not report['errors']:
        reference, candidate = values
        report['case_count'] = len(reference['records'])
        report['binaries'] = reference['binaries']
        keys = ('exit_code', 'stdout_sha256', 'stdout_len', 'stderr_sha256', 'stderr_len')
        for expected, actual in zip(reference['records'], candidate['records']):
            if all(expected.get(k) == actual.get(k) for k in keys):
                report['matched'] += 1
            else:
                report['mismatched'] += 1
                if len(report['differences']) < 50:
                    report['differences'].append({'id': expected['id'],
                                                  'fields': [k for k in keys if expected.get(k) != actual.get(k)],
                                                  'reference': {k: expected.get(k) for k in keys + ('stdout_prefix', 'stderr_prefix')},
                                                  'candidate': {k: actual.get(k) for k in keys + ('stdout_prefix', 'stderr_prefix')}})
        report['differences_truncated'] = report['mismatched'] > len(report['differences'])
        report['status'] = 'different' if report['mismatched'] else 'equal'
    write_json(output, report)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    cap = sub.add_parser('capture', help='explicitly execute the corpus and record per-case results')
    cap.add_argument('--corpus', default=str(ROOT/'tests/differential/cli_cases.json'))
    cap.add_argument('--output', required=True, type=Path)
    cap.add_argument('--adapter', type=Path)
    cap.add_argument('--timeout', default=30, type=int)
    cmp_ = sub.add_parser('compare', help='require a native Darwin reference and compare the same binaries')
    cmp_.add_argument('--reference', required=True, type=Path)
    cmp_.add_argument('--candidate', required=True, type=Path)
    cmp_.add_argument('--output', required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == 'capture':
            if not 1 <= args.timeout <= 600:
                raise ValueError('timeout must be in 1..600 seconds')
            value = capture(args.corpus, args.output, args.adapter, args.timeout)
            print(f"{value['status']}: {len(value['records'])} cases; evidence {args.output}")
            return 0 if value['status'] == 'captured' else 2
        value = compare(args.reference, args.candidate, args.output)
        print(f"{value['status']}: {value['matched']} matched, {value['mismatched']} mismatched; report {args.output}")
        return 0 if value['status'] == 'equal' else 1 if value['status'] == 'different' else 2
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        print(f'cli differential: {exc}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
