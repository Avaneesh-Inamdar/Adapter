"""Unmodified, hash-pinned BurntSushi ripgrep executable; behavior assertions.

No rebuilt ripgrep or Windows ripgrep participates. These are Windows
observations, not comparisons with a reference Mac. Fetch with
tools/fetch_ripgrep.py on a fresh clone.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from test_runtime import BIN, FIX

RG = FIX/'reference/ripgrep-macos-amd64'
SHA256 = '0c9a0066db0d26b640777db88045b0ccdd58509a746700e43e1c4ff8707a5ed0'


class RipgrepTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not RG.exists():
            raise unittest.SkipTest('run python tools/fetch_ripgrep.py to fetch the published target')
        if hashlib.sha256(RG.read_bytes()).hexdigest() != SHA256:
            raise AssertionError('ripgrep reference artifact hash mismatch')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name)
        self.write('sample.txt', 'alpha needle\nbeta\ngamma needle tail\n')
        (self.dir/'sub').mkdir()
        self.write('sub/nested.txt', 'nested needle here\n')
        self.write('sub/skip.log', 'ignore needle\n')
        self.write('.gitignore', 'skip.log\n')
        self.write('.hidden.txt', 'hidden needle\n')
        (self.dir/'.git').mkdir()

    def write(self, name, text):
        (self.dir/name).write_bytes(text.encode())

    def rg(self, *args, data=None, timeout=30):
        return subprocess.run(
            [str(BIN/'adapter-run.exe'), '--timeout', str(timeout), str(RG), '--', *args],
            cwd=self.dir, input=data, capture_output=True, timeout=timeout+5)

    def test_version_and_help(self):
        p = self.rg('--version')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertTrue(p.stdout.startswith(b'ripgrep 15.2.0 (rev e89fff89ac)'), p.stdout)
        self.assertIn(b'+pcre2', p.stdout)
        p = self.rg('--help')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn(b'recursively searches the current directory', p.stdout)

    def test_literal_and_regex_search(self):
        p = self.rg('-n', 'needle', 'sample.txt')
        self.assertEqual((p.returncode, p.stdout),
                         (0, b'1:alpha needle\n3:gamma needle tail\n'), p.stderr)
        p = self.rg('-n', '-C', '1', 'tail$', 'sample.txt')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout, b'2-beta\n3:gamma needle tail\n')

    def test_recursive_ignore_and_hidden(self):
        p = self.rg('needle', '.')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn(b'nested.txt:nested needle here', p.stdout)
        self.assertNotIn(b'skip.log', p.stdout)
        self.assertNotIn(b'.hidden.txt', p.stdout)
        p = self.rg('--hidden', '-l', 'needle', '.')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn(b'./.hidden.txt', p.stdout)
        self.assertNotIn(b'./.git/', p.stdout)

    def test_json_stream(self):
        p = self.rg('--json', 'needle', 'sample.txt')
        self.assertEqual(p.returncode, 0, p.stderr)
        events = [json.loads(line) for line in p.stdout.splitlines()]
        matches = [e for e in events if e['type'] == 'match']
        self.assertEqual(len(matches), 2)
        self.assertEqual(matches[0]['data']['line_number'], 1)
        self.assertEqual(matches[0]['data']['submatches'][0]['start'], 6)
        self.assertEqual(events[-1]['type'], 'summary')

    def test_pcre2_and_unicode(self):
        p = self.rg('-P', 'n(eed)le', 'sample.txt')
        self.assertEqual((p.returncode, p.stdout.count(b'needle')), (0, 2), p.stderr)
        self.write('utf8.txt', 'café needle\nΑλφα needle\n')
        p = self.rg('-n', 'Αλφα', 'utf8.txt')
        self.assertEqual((p.returncode, p.stdout), (0, '2:Αλφα needle\n'.encode()), p.stderr)

    def test_stdin_is_preferred_over_cwd(self):
        p = self.rg('-n', 'needle', data=b'pipe needle\nno match\n')
        self.assertEqual((p.returncode, p.stdout), (0, b'1:pipe needle\n'), p.stderr)

    def test_binary_detection_and_text_mode(self):
        (self.dir/'binary.bin').write_bytes(b'abc\0needle\n')
        p = self.rg('needle', 'binary.bin')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn(b'binary file matches', p.stdout)
        p = self.rg('-a', 'needle', 'binary.bin')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout, b'abc\0needle\n')

    def test_large_file_memory_map(self):
        payload = b'x' * (2 * 1024 * 1024) + b' needle at the end\n'
        (self.dir/'large.txt').write_bytes(payload)
        p = self.rg('-o', 'needle', 'large.txt')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout, b'needle\n')

    def test_multithreaded_directory_search(self):
        many = self.dir/'many'
        many.mkdir()
        for index in range(40):
            (many/f'f{index}.txt').write_bytes(f'needle in file {index}\n'.encode())
        p = self.rg('-l', 'needle', 'many')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(len(p.stdout.splitlines()), 40)

    def test_exit_codes_and_regex_errors(self):
        self.assertEqual(self.rg('absent', 'sample.txt').returncode, 1)
        p = self.rg('(', 'sample.txt')
        self.assertEqual(p.returncode, 2)
        self.assertIn(b'regex parse error', p.stderr)

    def test_preprocessor_spawn_fails_cleanly(self):
        p = self.rg('--pre', 'cmd /c type', 'needle', 'sample.txt')
        self.assertEqual(p.returncode, 2)
        self.assertIn(b'preprocessor command could not start', p.stderr)


if __name__ == '__main__':
    unittest.main()
