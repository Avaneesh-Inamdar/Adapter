"""Unmodified, hash-pinned jqlang macOS executable; expected behavior assertions.

No rebuilt jq or Windows jq participates. These are Windows observations, not
comparisons with a reference Mac. Fetch with tools/fetch_jq.py on a fresh clone.
"""
import hashlib
import json
import math
import os
from pathlib import Path
import random
import subprocess
import tempfile
import unittest
from test_runtime import BIN, FIX

JQ = FIX/'reference/jq-macos-amd64'
SHA256 = 'e94b266e3c26690550006abe63152b782280f4e14374accdf04cbde844f00bc0'


class JqTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not JQ.exists():
            raise unittest.SkipTest('run python tools/fetch_jq.py to fetch the published target')
        if hashlib.sha256(JQ.read_bytes()).hexdigest() != SHA256:
            raise AssertionError('jq reference artifact hash mismatch')

    def jq(self, *args, data='', env=None):
        result = subprocess.run([str(BIN/'adapter-run.exe'), '--timeout', '10', str(JQ), '--', *args],
                                input=data.encode('utf-8'), capture_output=True,
                                env={**os.environ, **(env or {})}, timeout=15)
        result.stdout = result.stdout.decode('utf-8'); result.stderr = result.stderr.decode('utf-8')
        return result

    def value(self, expression, data='', *args):
        p = self.jq(*args, '-c', expression, data=data)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stderr, '')
        return [json.loads(line) for line in p.stdout.splitlines()]

    def test_version_and_help(self):
        p = self.jq('--version'); self.assertEqual((p.returncode, p.stdout, p.stderr), (0, 'jq-1.8.2\n', ''))
        p = self.jq('--help'); self.assertEqual(p.returncode, 0, p.stderr); self.assertIn('jq - commandline JSON processor', p.stdout)

    def test_piped_json_transform(self):
        self.assertEqual(self.value('{x:.a|map(. * 2)}', '{"a":[1,2,3]}\n'), [{'x': [2,4,6]}])

    def test_large_stream_and_reduction(self):
        items = [{'v': i, 'keep': i%3 == 0} for i in range(6000)]
        data = ''.join(json.dumps(v)+'\n' for v in items)
        self.assertEqual(self.value('map(select(.keep)|.v) | add', data, '-s'),
                         [sum(v['v'] for v in items if v['keep'])])

    def test_seeded_sort_group_and_arithmetic(self):
        rng = random.Random(13829); values = [rng.randrange(-1000000,1000000) for _ in range(400)]
        self.assertEqual(self.value('{sorted:sort, sum:add, groups:group_by(.%7)|map(length)}', json.dumps(values)),
                         [{'sorted': sorted(values), 'sum':sum(values),
                           'groups': [sum(int(math.fmod(v,7))==i for v in values) for i in range(-6,7)]}])

    def test_unicode_regex_and_escaping(self):
        self.assertEqual(self.value('capture("(?<word>[a-z]+)(?<digits>[0-9]+)")', '"abc123"'),
                         [{'word':'abc','digits':'123'}])
        p = self.jq('-nr', '"héllo 世界 🚀"'); self.assertEqual(p.returncode, 0, p.stderr); self.assertEqual(p.stdout, 'héllo 世界 🚀\n')
        p = self.jq('-na', '"世界"'); self.assertEqual(p.returncode, 0, p.stderr); self.assertIn('\\u4e16\\u754c', p.stdout)

    def test_raw_binary_input_preserves_crlf_and_nul(self):
        self.assertEqual(self.value('.', 'A\r\nB\0C\n', '-Rs'), ['A\r\nB\0C\n'])

    def test_utf8_file_slurp_and_rawfile(self):
        with tempfile.TemporaryDirectory(prefix='db-世界-') as temp:
            path = Path(temp)/'données.json'; path.write_text('{"name":"café"}\n{"name":"茶"}\n', encoding='utf-8')
            p = self.jq('-sc', 'map(.name)', str(path)); self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(json.loads(p.stdout), ['café','茶'])
            p = self.jq('-ncr', '--rawfile', 'data', str(path), '$data'); self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(p.stdout, path.read_bytes().decode('utf-8')+'\n')

    def test_filter_file_and_module_search(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp); (folder/'double.jq').write_text('def double: . * 2;\n')
            script = folder/'main.jq'; script.write_text('include "double"; [range(5)] | map(double)\n')
            guest_folder = '/'+folder.drive[0]+folder.as_posix()[2:]
            p = self.jq('-nc', '-L', guest_folder, '-f', str(script)); self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(json.loads(p.stdout), [0,2,4,6,8])

    def test_missing_file_and_directory_errors(self):
        with tempfile.TemporaryDirectory() as temp:
            p = self.jq('.', str(Path(temp)/'missing.json')); self.assertEqual(p.returncode, 2); self.assertIn('No such file', p.stderr)
            p = self.jq('.', temp); self.assertEqual(p.returncode, 2); self.assertIn('directory', p.stderr.lower())

    def test_parse_compile_and_exit_statuses(self):
        p = self.jq('.', data='{"bad":'); self.assertEqual(p.returncode, 5); self.assertIn('parse error', p.stderr)
        p = self.jq('-n', 'bad_syntax('); self.assertEqual(p.returncode, 3); self.assertIn('syntax error', p.stderr)
        p = self.jq('-ne', 'false'); self.assertEqual(p.returncode, 1)
        p = self.jq('-ne', 'empty'); self.assertEqual(p.returncode, 4)
        p = self.jq('-ne', 'true'); self.assertEqual(p.returncode, 0)
        p = self.jq('-n', 'halt_error(7)'); self.assertEqual(p.returncode, 7)

    def test_utc_time_parse_format_and_negative_epoch(self):
        self.assertEqual(self.value('fromdateiso8601 | todateiso8601', '"2024-02-29T12:34:56Z"'), ['2024-02-29T12:34:56Z'])
        self.assertEqual(self.value('[0, -86400, 951782400] | map(strftime("%Y-%m-%dT%H:%M:%SZ"))', '', '-n'),
                         [['1970-01-01T00:00:00Z','1969-12-31T00:00:00Z','2000-02-29T00:00:00Z']])
        self.assertEqual(self.value('strptime("%Y-%m-%d") | .[0:3]', '"2000-02-29"'), [[2000,1,29]])

    def test_math_and_extended_precision_argument(self):
        values = self.value('[(0|sin), pow(2;10), nexttoward(1;2), (1|lgamma)]', '', '-n')[0]
        self.assertEqual(values[:3], [0,1024,math.nextafter(1,2)])
        self.assertAlmostEqual(values[3], 0)

    def test_environment_and_cli_variables(self):
        p = self.jq('-nc', '[env.DB_JQ_UNICODE, $name, $value]', '--arg', 'name', '世界', '--argjson', 'value', '{"a":3}', env={'DB_JQ_UNICODE':'café'})
        self.assertEqual(p.returncode, 0, p.stderr); self.assertEqual(json.loads(p.stdout), ['café','世界',{'a':3}])


if __name__ == '__main__':
    unittest.main()
