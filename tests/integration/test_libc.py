"""Actual Darwin ABI probes, executed by the Windows worker."""
import subprocess
import tempfile
from pathlib import Path
import unittest
from test_runtime import BIN, FIX


class LibcTests(unittest.TestCase):
    def test_utf8_file_stat_errors_and_utc_time(self):
        with tempfile.TemporaryDirectory(prefix='db-世界-') as folder:
            path = Path(folder)/'données.bin'
            guest = '/'+path.drive[0]+path.as_posix()[2:]
            p = subprocess.run([str(BIN/'db-run.exe'), str(FIX/'file_time'), '--', guest],
                               input=b'', capture_output=True, timeout=15)
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(p.stdout, b'libc:UTF-8 files, stat64, errno and UTC time passed\n')
            self.assertEqual(path.read_bytes(), b'one\r\ntwo\n\0\xff\0')

    def test_locale_streams_varargs_and_extended_precision(self):
        for name in ['libc', 'libc-chained']:
            with self.subTest(name=name):
                p = subprocess.run([str(BIN/'db-run.exe'), str(FIX/name)],
                                   input=b'A\r\n\0\xffZ', capture_output=True, timeout=15)
                self.assertEqual(p.returncode, 0, p.stderr)
                self.assertEqual(p.stdout, b'libc:printf 7 2.5\nlibc:slow\nlibc:locale, stdio, varargs and long double passed\n')
                self.assertEqual(p.stderr, b'libc:stderr 4294967297 1.25\n')

    def test_explicit_exit_finalizes_and_flushes(self):
        p = subprocess.run([str(BIN/'db-run.exe'), str(FIX/'libc-chained'), '--', 'exit'],
                           input=b'', capture_output=True, timeout=15)
        self.assertEqual(p.returncode, 23, p.stderr)
        self.assertEqual(p.stdout, b'libc:exit callback\n')


if __name__ == '__main__':
    unittest.main()
