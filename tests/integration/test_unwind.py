"""Guest dyld section queries, including dynamic images and malformed metadata."""
import json
import struct
import unittest
import test_runtime as runtime

FIX, call = runtime.FIX, runtime.call


class UnwindTests(unittest.TestCase):
    def run_query(self, name, plugin=None):
        arguments = [FIX/name]
        if plugin: arguments += ['--', str(FIX/plugin)]
        p = call('adapter-run', *arguments)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stderr, '')
        lines = p.stdout.splitlines()
        self.assertEqual(lines[0], 'unwind:constructor')
        records = [json.loads(line) for line in lines[1:]]
        self.assertEqual([r['index'] for r in records], list(range(8)))
        for index in [0, 1, 3, 6]: self.assertEqual(records[index]['result'], [1, 1, 1, 1])
        for index in [2, 7]: self.assertEqual(records[index]['result'], [0, 1, 1, 1])
        return records

    def test_startup_compact_dwarf_and_absent_sections(self):
        for mode in ['compact', 'dwarf', 'none']:
            for suffix in ['', '-chained']:
                with self.subTest(mode=mode, suffix=suffix):
                    records = self.run_query('unwind-' + mode + suffix)
                    if mode == 'none':
                        self.assertEqual(records[4]['result'], [1, 0, 0, 0])
                        self.assertEqual(records[5]['result'], [1, 0, 0, 0])
                    else:
                        self.assertEqual(records[4]['result'][:2], [1, 1])
                        self.assertEqual(records[5]['result'][:2], [1, 1])
                        self.assertEqual(records[5]['result'][3], 1)
                        if mode == 'dwarf': self.assertGreater(records[4]['result'][2], 24)

    def test_dlopen_images_registered_before_plugin_constructor(self):
        for mode in ['compact', 'dwarf']:
            for suffix in ['', '-chained']:
                with self.subTest(mode=mode, suffix=suffix):
                    records = self.run_query('unwind-compact', 'libunwind-' + mode + suffix + '.dylib')
                    self.assertGreater(records[5]['result'][2], 0)
                    if mode == 'dwarf': self.assertGreater(records[4]['result'][2], 24)

    def reject_mutation(self, mutate, diagnostic):
        helper = runtime.RuntimeTests(); helper.addCleanup = self.addCleanup
        path = helper.mutate('unwind-compact', mutate)
        p = call('adapter-run', path)
        self.assertEqual(p.returncode, 125, p.stderr)
        self.assertEqual(p.stdout, '')
        self.assertIn(diagnostic, p.stderr)

    @staticmethod
    def section_header(data, name):
        commands_end = 32 + struct.unpack_from('<I', data, 20)[0]
        return data.index(name.encode().ljust(16, b'\0'), 32, commands_end)

    def test_section_file_offset_must_match_mapped_bytes(self):
        def mutate(data):
            header = self.section_header(data, '__unwind_info')
            offset = struct.unpack_from('<I', data, header + 48)[0]
            struct.pack_into('<I', data, header + 48, offset + 1)
        self.reject_mutation(mutate, 'outside file-backed __TEXT')

    def test_section_cannot_escape_text(self):
        def mutate(data):
            header = self.section_header(data, '__unwind_info')
            struct.pack_into('<Q', data, header + 32, 0x100000000 - 8)
        self.reject_mutation(mutate, 'section exceeds segment VM range')

    def test_duplicate_unwind_sections_rejected(self):
        def mutate(data):
            header = self.section_header(data, '__eh_frame')
            data[header:header + 16] = b'__unwind_info'.ljust(16, b'\0')
        self.reject_mutation(mutate, 'duplicate unwind metadata section')


if __name__ == '__main__':
    unittest.main()
