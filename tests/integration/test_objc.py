"""Compiler-emitted Apple metadata, native SysV dispatch and rejection gates."""
import struct
import unittest
import test_runtime as runtime
FIX, call = runtime.FIX, runtime.call


class ObjcTests(unittest.TestCase):
    def test_protocols_properties_and_accessors(self):
        expected = ('objc:metadata constructor\n'
                    'objc:protocol inheritance, descriptions, properties and accessors passed\n')
        for name in ['objc_metadata', 'objc_metadata-chained', 'objc_metadata-relative']:
            with self.subTest(name=name):
                p = call('adapter-run', FIX/name)
                self.assertEqual(p.returncode, 0, p.stderr)
                self.assertEqual(p.stdout, expected)
                self.assertEqual(p.stderr, '')

    def metadata_mutation(self, symbol, relative, format, value, diagnostic):
        helper = runtime.RuntimeTests(); helper.addCleanup = self.addCleanup
        image = helper.inspect('objc_metadata')
        address = next(s['address'] for s in image['symbols'] if s['name'] == symbol)
        segment = next(s for s in image['segments'] if s['vmaddr'] <= address < s['vmaddr'] + s['filesize'])
        offset = segment['fileoff'] + address - segment['vmaddr'] + relative
        path = helper.mutate('objc_metadata', lambda d: struct.pack_into(format, d, offset, value))
        p = call('adapter-run', path)
        self.assertEqual(p.returncode, 125, p.stderr)
        self.assertEqual(p.stdout, '')
        self.assertIn(diagnostic, p.stderr)

    def test_protocol_size_and_flags_rejected_before_constructor(self):
        for offset, value in [(64, 104), (68, 1)]:
            self.metadata_mutation('__OBJC_PROTOCOL_$_MetadataDerived', offset, '<I', value,
                                   'protocol size, flags or preinitialized isa')

    def test_protocol_count_rejected_before_constructor(self):
        self.metadata_mutation('__OBJC_$_PROTOCOL_REFS_MetadataDerived', 0, '<Q', 8193,
                               'protocol list limit')

    def test_protocol_implementation_rejected_before_constructor(self):
        self.metadata_mutation('__OBJC_$_PROTOCOL_INSTANCE_METHODS_MetadataBase', 24, '<Q', 1,
                               'invalid protocol method description')

    def test_property_count_rejected_before_constructor(self):
        self.metadata_mutation('__OBJC_$_PROP_LIST_MetadataRoot', 4, '<I', 65537,
                               'property limit')

    def test_protocol_cycle_rejected_before_constructor(self):
        helper = runtime.RuntimeTests(); helper.addCleanup = self.addCleanup
        image = helper.inspect('objc_metadata')
        address = next(s['address'] for s in image['symbols'] if s['name'] == '__OBJC_PROTOCOL_$_MetadataDerived')
        self.metadata_mutation('__OBJC_$_PROTOCOL_REFS_MetadataDerived', 8, '<Q', address,
                               'protocol inheritance cycle')

    def test_core_classic_and_chained(self):
        expected = ('objc:constructor\n'
                    'objc:metadata, inheritance, initialize, GP/SSE/stack, nil and ownership passed\n')
        for name in ['objc_core', 'objc_core-chained', 'objc_core-relative']:
            p = call('adapter-run', FIX/name)
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(p.stdout, expected)
            self.assertEqual(p.stderr, '')

    def test_categories_override_and_extend(self):
        expected = 'objc:category methods override and extend the class\n'
        for name in ['objc_category', 'objc_category-chained', 'objc_category-relative']:
            p = call('adapter-run', FIX/name)
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(p.stdout, expected)
            self.assertEqual(p.stderr, '')

    def test_invalid_method_count_rejected_before_constructor(self):
        helper = runtime.RuntimeTests()
        # Reuse the existing bounded file-mutation utility without inheriting
        # its entire unittest class into this test module.
        helper.addCleanup = self.addCleanup
        image = helper.inspect('objc_core')
        const = next(s for s in image['sections'] if s['name']=='__objc_const')
        raw = (FIX/'objc_core').read_bytes()
        offset = raw.index(struct.pack('<II', 24, 5), const['offset'], const['offset']+const['size'])
        path = helper.mutate('objc_core', lambda d: struct.pack_into('<I', d, offset+4, 65537))
        p = call('adapter-run', path)
        self.assertEqual(p.returncode, 125)
        self.assertEqual(p.stdout, '')
        self.assertIn('Objective-C method limit', p.stderr)

    def test_image_flags_rejected_before_constructor(self):
        helper = runtime.RuntimeTests(); helper.addCleanup = self.addCleanup
        image = helper.inspect('objc_core')
        info = next(s for s in image['sections'] if s['name']=='__objc_imageinfo')
        path = helper.mutate('objc_core', lambda d: struct.pack_into('<I', d, info['offset']+4, 0x41))
        p = call('adapter-run', path)
        self.assertEqual(p.returncode, 125)
        self.assertEqual(p.stdout, '')
        self.assertIn('Objective-C image flags', p.stderr)


if __name__ == '__main__':
    unittest.main()
