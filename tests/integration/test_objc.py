"""Compiler-emitted Apple metadata, native SysV dispatch and rejection gates."""
import struct
import unittest
import test_runtime as runtime
FIX, call = runtime.FIX, runtime.call


class ObjcTests(unittest.TestCase):
    def test_core_classic_and_chained(self):
        expected = ('objc:constructor\n'
                    'objc:metadata, inheritance, initialize, GP/SSE/stack, nil and ownership passed\n')
        for name in ['objc_core', 'objc_core-chained']:
            p = call('adapter-run', FIX/name)
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(p.stdout, expected)
            self.assertEqual(p.stderr, '')

    def test_categories_rejected_before_constructor(self):
        p = call('adapter-run', FIX/'objc_category')
        self.assertEqual(p.returncode, 125)
        self.assertEqual(p.stdout, '')
        self.assertIn('__objc_catlist', p.stderr)

    def test_relative_lists_rejected_before_constructor(self):
        p = call('adapter-run', FIX/'objc_core-relative')
        self.assertEqual(p.returncode, 125)
        self.assertEqual(p.stdout, '')
        self.assertIn('Objective-C', p.stderr)

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
