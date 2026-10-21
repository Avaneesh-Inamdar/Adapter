"""Evidence validation for the CLI differential; synthetic captures are not Mac results."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tools'))
import cli_differential as cli

CORPUS = Path(__file__).resolve().parents[1] / 'differential' / 'cli_cases.json'


def corpus():
    return json.loads(CORPUS.read_text(encoding='utf-8'))


def record(identifier, exit_code=0, out='', err=''):
    return {'id': identifier, 'status': 'captured', 'exit_code': exit_code,
            'stdout_sha256': cli.sha256(out.encode()), 'stdout_len': len(out.encode()),
            'stderr_sha256': cli.sha256(err.encode()), 'stderr_len': len(err.encode()),
            'stdout_prefix': out, 'stderr_prefix': err}


def capture(native=True, records=None):
    value = corpus()
    records = records if records is not None else [record(case['id']) for case in value['cases']]
    return {'schema': 1, 'suite': cli.SUITE, 'mode': 'native' if native else 'adapter',
            'host': {'system': 'Darwin' if native else 'Windows', 'release': 'x', 'machine': 'x86_64',
                     'python': '3.12.10'},
            'adapter': None if native else {'sha256': 'd' * 64, 'path': 'adapter-run.exe'},
            'binaries': {'jq': {'file': 'jq-macos-amd64', 'bytes': 1, 'sha256': 'a' * 64}},
            'corpus': {'file': 'cli_cases.json', 'sha256': 'b' * 64},
            'child_environment': cli.CHILD_ENV, 'timeout_seconds': 30,
            'status': 'captured', 'errors': [], 'records': records}


class CliDifferentialTests(unittest.TestCase):
    def compare(self, reference, candidate):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            cli.write_json(root / 'reference.json', reference)
            cli.write_json(root / 'candidate.json', candidate)
            return cli.compare(root / 'reference.json', root / 'candidate.json', root / 'report.json')

    def test_committed_corpus_is_valid_and_names_both_programs(self):
        self.assertEqual(cli.corpus_errors(corpus()), [])
        binaries = {case['binary'] for case in corpus()['cases']}
        self.assertEqual(binaries, {'jq', 'rg'})
        identifiers = [case['id'] for case in corpus()['cases']]
        self.assertEqual(len(identifiers), len(set(identifiers)))

    def test_corpus_rejects_duplicate_ids_unknown_binaries_and_bad_argv(self):
        value = corpus()
        value['cases'].append(copy.deepcopy(value['cases'][0]))
        self.assertTrue(any('duplicate' in error for error in cli.corpus_errors(value)))
        value = corpus()
        value['cases'][0]['binary'] = 'python'
        self.assertTrue(any('unknown binary' in error for error in cli.corpus_errors(value)))
        value = corpus()
        value['cases'][0]['argv'] = [1, 2]
        self.assertTrue(any('invalid argv' in error for error in cli.corpus_errors(value)))
        value = corpus()
        value['suite'] = 'adapter-other-v1'
        self.assertTrue(any('suite mismatch' in error for error in cli.corpus_errors(value)))

    def test_same_evidence_compares_equal_including_adapter_capture(self):
        result = self.compare(capture(), capture(native=False))
        self.assertEqual(result['status'], 'equal')
        self.assertEqual((result['matched'], result['mismatched'], result['case_count']), (30, 0, 30))
        self.assertEqual(result['errors'], [])

    def test_any_field_difference_is_a_mismatch_with_detail(self):
        candidate = capture(native=False)
        candidate['records'][3] = record(candidate['records'][3]['id'], exit_code=1, out='x')
        result = self.compare(capture(), candidate)
        self.assertEqual(result['status'], 'different')
        self.assertEqual(result['mismatched'], 1)
        self.assertEqual(result['differences'][0]['fields'], ['exit_code', 'stdout_sha256', 'stdout_len'])
        self.assertEqual(result['differences'][0]['candidate']['stdout_prefix'], 'x')

    def test_windows_capture_is_never_accepted_as_the_reference(self):
        result = self.compare(capture(native=False), capture(native=False))
        self.assertEqual(result['status'], 'invalid')
        self.assertTrue(any('reference must be a native Darwin capture' in error for error in result['errors']))

    def test_corpus_or_binary_drift_invalidates_the_comparison(self):
        candidate = capture(native=False)
        candidate['corpus'] = {'file': 'cli_cases.json', 'sha256': 'c' * 64}
        self.assertEqual(self.compare(capture(), candidate)['status'], 'invalid')
        candidate = capture(native=False)
        candidate['binaries'] = {'jq': {'file': 'jq-macos-amd64', 'bytes': 2, 'sha256': 'e' * 64}}
        self.assertEqual(self.compare(capture(), candidate)['status'], 'invalid')
        candidate = capture(native=False)
        candidate['records'] = list(reversed(candidate['records']))
        self.assertEqual(self.compare(capture(), candidate)['status'], 'invalid')

    def test_incomplete_capture_is_invalid(self):
        broken = capture(native=False)
        broken['status'] = 'invalid'
        broken['errors'] = ['jq-version timed out']
        broken['records'][5] = {'id': 'jq-unicode', 'status': 'timed-out', 'timeout_seconds': 30,
                                'stdout_len': 0, 'stderr_len': 0}
        result = self.compare(capture(), broken)
        self.assertEqual(result['status'], 'invalid')
        self.assertTrue(any('without a result' in error for error in result['errors']))


if __name__ == '__main__':
    unittest.main()
