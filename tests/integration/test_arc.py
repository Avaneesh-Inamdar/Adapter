"""Actual ARC lowering, ownership lifetimes and pre-constructor admission."""
import json
import struct
import unittest
import test_runtime as runtime

FIX, call = runtime.FIX, runtime.call


class ArcTests(unittest.TestCase):
    def test_arc_ownership_at_o0_o2_with_all_fixup_forms(self):
        expected = [[1, 1, 1, 2], [1, 1, 0, 1], [1, 1, 1, 1], [2, 42, 0, 3],
                    [42, 1, 2, 2], [1, 2, 1, 1], [42, 1, 0, 1], [0, 0, 1, 1]]
        for level in ['o0', 'o2']:
            for suffix in ['', '-chained', '-relative']:
                with self.subTest(level=level, suffix=suffix):
                    p = call('adapter-run', FIX/('objc_arc-' + level + suffix))
                    self.assertEqual(p.returncode, 0, p.stderr)
                    self.assertEqual(p.stderr, '')
                    lines = p.stdout.splitlines()
                    self.assertEqual(lines[0], 'objc:ARC constructor')
                    records = [json.loads(line) for line in lines[1:]]
                    self.assertEqual([r['index'] for r in records], list(range(8)))
                    self.assertEqual([r['result'] for r in records], expected)

    def mutate(self, symbol, relative, format, value, diagnostic):
        helper = runtime.RuntimeTests(); helper.addCleanup = self.addCleanup
        image = helper.inspect('objc_arc-o0')
        address = next(s['address'] for s in image['symbols'] if s['name'] == symbol)
        def file_offset(vmaddr):
            segment = next(s for s in image['segments'] if s['vmaddr'] <= vmaddr < s['vmaddr'] + s['filesize'])
            return segment['fileoff'] + vmaddr - segment['vmaddr']
        offset = file_offset(address)
        if isinstance(relative, tuple):
            raw = (FIX/'objc_arc-o0').read_bytes()
            address = struct.unpack_from('<Q', raw, offset + relative[0])[0]
            offset = file_offset(address) + relative[1]
        else:
            offset += relative
        path = helper.mutate('objc_arc-o0', lambda d: struct.pack_into(format, d, offset, value))
        p = call('adapter-run', path)
        self.assertEqual(p.returncode, 125, p.stderr)
        self.assertEqual(p.stdout, '')
        self.assertIn(diagnostic, p.stderr)

    def test_cxx_constructor_metadata_is_still_rejected(self):
        self.mutate('__OBJC_CLASS_RO_$_ArcNode', 0, '<I', 132, 'class flags')

    def test_overlapping_strong_and_weak_maps_rejected(self):
        self.mutate('__OBJC_CLASS_RO_$_ArcNode', (56, 0), '<B', 1, 'ownership layouts overlap')

    def test_ownership_map_outside_instance_rejected(self):
        self.mutate('__OBJC_CLASS_RO_$_ArcNode', (16, 0), '<B', 0xf1, 'ownership layout outside instance')

    def test_ownership_map_without_destructor_rejected(self):
        self.mutate('__OBJC_CLASS_RO_$_ArcNode', 0, '<I', 128, 'ownership layout without destructor')

    def test_object_ivar_cannot_overlap_isa(self):
        self.mutate('_OBJC_IVAR_$_ArcNode._child', 0, '<I', 0, 'object ivar ownership or invalid layout')


if __name__ == '__main__':
    unittest.main()
