"""Black-box tests: actual PE executables loading actual cross-linked Mach-O."""
import json
import os
from pathlib import Path
import plistlib
import struct
import subprocess
import tempfile
import shutil
import unittest

ROOT=Path(__file__).resolve().parents[2]
FIX=ROOT/"tests/fixtures/generated"
BIN=Path(os.environ.get("ADAPTER_BIN",ROOT/"target/debug"))

def call(tool,*args,timeout=15):
    return subprocess.run([str(BIN/(tool+".exe")),*map(str,args)],text=True,capture_output=True,timeout=timeout)

class RuntimeTests(unittest.TestCase):
    def inspect(self,name):
        p=call("adapter-inspect","--json",FIX/name)
        self.assertEqual(p.returncode,0,p.stderr)
        return json.loads(p.stdout)
    def mutate(self,name,fn):
        data=bytearray((FIX/name).read_bytes());fn(data)
        temp=tempfile.NamedTemporaryFile(delete=False,suffix=".macho");temp.write(data);temp.close()
        self.addCleanup(lambda:Path(temp.name).unlink(missing_ok=True))
        return Path(temp.name)
    def test_inspect_thin(self):
        d=self.inspect("hello");self.assertEqual(d["architecture"],"x86_64");self.assertTrue(d["entry_file_offset"]);self.assertIn("_puts",[s["name"] for s in d["symbols"]]);self.assertEqual(d["dependencies"][0]["name"],"/usr/lib/libSystem.B.dylib")
    def test_inspect_universal(self):
        d=self.inspect("universal");self.assertEqual(len(d["slices"]),2);self.assertEqual(d["selected_slice_offset"],4096)
    def test_inspect_arm64(self):
        d=self.inspect("arm64.o");self.assertEqual(d["architecture"],"arm64");self.assertIn("execution requires x86_64",d["unsupported"])
    def test_execute_arm64_rejected(self):
        p=call("adapter-run","--check",FIX/"arm64.o");self.assertEqual(p.returncode,125)
    def test_return_value(self):self.assertEqual(call("adapter-run",FIX/"return37").returncode,37)
    def test_universal_executes(self):self.assertEqual(call("adapter-run",FIX/"universal").returncode,37)
    def test_hello(self):
        p=call("adapter-run","--trace",FIX/"hello","--","hello");self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(p.stdout,"Adapter: Mach-O code executed on Windows\n");self.assertIn("[initialize]",p.stderr);self.assertIn("releasing mapped image",p.stderr)
    def test_wrong_argv(self):self.assertEqual(call("adapter-run",FIX/"hello","--","wrong").returncode,93)
    def test_corefoundation(self):
        p=call("adapter-run",FIX/"corefoundation");self.assertEqual(p.returncode,0,p.stderr);self.assertIn("UTF-8 / UTF-16 checks passed",p.stdout)
    def test_chained_hello(self):
        d=self.inspect("hello-chained");self.assertIsNotNone(d["chained_fixups"]);p=call("adapter-run",FIX/"hello-chained","--","hello");self.assertEqual(p.returncode,0,p.stderr);self.assertIn("code executed",p.stdout)
    def test_chained_corefoundation(self):
        p=call("adapter-run",FIX/"corefoundation-chained");self.assertEqual(p.returncode,0,p.stderr);self.assertIn("UTF-8 / UTF-16 checks passed",p.stdout)
    def test_bad_chained_version(self):
        d=self.inspect("hello-chained");offset=d["chained_fixups"]["offset"];path=self.mutate("hello-chained",lambda data:struct.pack_into("<I",data,offset,1));p=call("adapter-run","--check",path);self.assertEqual(p.returncode,125);self.assertIn("chained fixups version",p.stderr)
    def test_abi_internal_float_and_stack_arguments(self):
        p=call("adapter-run",FIX/"abi");self.assertEqual(p.returncode,0,p.stderr)
    def test_bundle(self):
        p=call("adapter-run",FIX/"Hello.app","--","hello");self.assertEqual(p.returncode,0,p.stderr)
    def test_bundle_traversal_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            contents=Path(temp)/"Bad.app/Contents";contents.mkdir(parents=True)
            (contents/"Info.plist").write_bytes(plistlib.dumps({"CFBundleExecutable":"../escape"}))
            p=call("adapter-inspect",contents.parent);self.assertNotEqual(p.returncode,0);self.assertIn("invalid CFBundleExecutable",p.stderr)
    def test_preflight_does_not_execute(self):
        p=call("adapter-run","--check",FIX/"hello");self.assertEqual(p.returncode,0,p.stderr);self.assertIn("execution not attempted",p.stdout);self.assertNotIn("code executed",p.stdout)
    def test_unknown_symbol(self):
        p=call("adapter-run",FIX/"unknown");self.assertEqual(p.returncode,125);self.assertIn("unimplemented symbol _deliberately_missing",p.stderr);self.assertEqual(p.stdout,"")
    def test_dependency_graph(self):
        p=call("adapter-inspect","--deps",FIX/"hello");g=json.loads(p.stdout);self.assertEqual(g["unresolved_required"],0);self.assertTrue(g["executable_by_current_runtime"])
    def test_rpath_resolution(self):
        p=call("adapter-inspect","--deps",FIX/"local");self.assertEqual(p.returncode,0,p.stderr);g=json.loads(p.stdout);self.assertEqual(len(g["nodes"]),2);self.assertEqual(g["unresolved_required"],0);self.assertTrue(g["executable_by_current_runtime"])
    def test_local_dylib_execution(self):
        p=call("adapter-run","--check",FIX/"local");self.assertEqual(p.returncode,0,p.stderr)
        p=call("adapter-run","--trace",FIX/"local");self.assertEqual(p.returncode,7,p.stderr);self.assertIn("releasing mapped images: 2",p.stderr)
    def test_dylib_constructors_destructors_and_shared_data(self):
        expected='leaf:init\nmiddle:init\nmain:init\ndylib:passed\nmain:fini\nmiddle:fini\nleaf:fini\n'
        for variant in ['classic','chained']:
            with self.subTest(variant=variant):
                p=call('adapter-run','--trace',FIX/'dylibs'/variant/'multi');self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(p.stdout,expected);self.assertIn('releasing mapped images: 3',p.stderr)
    def test_reexported_dylib(self):
        for variant in ['classic','chained']:
            p=call('adapter-run',FIX/'dylibs'/variant/'facade');self.assertEqual(p.returncode,17,p.stderr);self.assertEqual(p.stdout,'leaf:init\nleaf:fini\n')
    def test_two_level_duplicate_symbols(self):
        p=call('adapter-run','--trace',FIX/'dylibs/namespace/namespace');self.assertEqual(p.returncode,0,p.stderr);self.assertIn('releasing mapped images: 5',p.stderr)
    def test_thread_local_storage_main_thread(self):
        for name in ['tls','tls-chained']:
            p=call('adapter-run',FIX/name);self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(p.stdout,'tls:passed\n')
    def test_tls_across_threads_and_images(self):
        for variant in ['classic','chained']:
            p=call('adapter-run',FIX/'dylibs'/variant/'tls-threads');self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(p.stdout,'tls:8 threads and 2 images passed\n')
    def test_tls_preserves_live_integer_sse_avx_registers(self):
        for variant in ['classic','chained']:
            p=call('adapter-run',FIX/'tls-abi'/variant);self.assertEqual(p.returncode,0,p.stderr);self.assertIn(p.stdout,['tls:register preservation passed\n','tls:register preservation passed (AVX unavailable)\n'])
    def test_thread_once_keys_destructors_and_errno(self):
        for name in ['thread_state','thread_state-chained']:
            p=call('adapter-run',FIX/name);self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(p.stdout,'threads:once, keys, destructors and errno passed\n')
    def test_dependency_cycle_fails_before_constructors(self):
        p=call('adapter-run',FIX/'dylibs/cycle/cycle');self.assertEqual(p.returncode,125);self.assertIn('dependency cycle',p.stderr);self.assertEqual(p.stdout,'')
    def test_exit_callback_filter_order_and_once(self):
        p=call('adapter-run',FIX/'exit_callbacks');self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(p.stdout,'callback:a\ncallback:a\ncallback:main\ncallback:last\ncallback:b\n')
    def test_exports_work_without_nlist(self):
        with tempfile.TemporaryDirectory() as temp:
            temp=Path(temp);shutil.copyfile(FIX/'local',temp/'local');shutil.copyfile(FIX/'libhelper.dylib',temp/'libhelper.dylib')
            d=self.inspect('libhelper.dylib');cmd=next(c for c in d['commands'] if c['code']==2)
            path=temp/'libhelper.dylib';data=bytearray(path.read_bytes());struct.pack_into('<I',data,cmd['offset']+12,0);path.write_bytes(data)
            p=call('adapter-run',temp/'local');self.assertEqual(p.returncode,7,p.stderr)
    def test_hidden_nlist_symbol_is_not_an_export(self):
        with tempfile.TemporaryDirectory() as temp:
            temp=Path(temp);shutil.copyfile(FIX/'local',temp/'local');shutil.copyfile(FIX/'libhelper.dylib',temp/'libhelper.dylib')
            d=self.inspect('libhelper.dylib');stream=d.get('export_trie') or d['dyld']['export'];path=temp/'libhelper.dylib';data=bytearray(path.read_bytes());start,size=stream['offset'],stream['size'];blob=bytes(data[start:start+size]);self.assertIn(b'_helper',blob);data[start:start+size]=blob.replace(b'_helper',b'_hidden');path.write_bytes(data)
            p=call('adapter-run',temp/'local');self.assertEqual(p.returncode,125);self.assertIn('unimplemented symbol _helper',p.stderr)
    def test_dependency_missing_import_prevents_all_initializers(self):
        with tempfile.TemporaryDirectory() as temp:
            dest=Path(temp)/'images';shutil.copytree(FIX/'dylibs/classic',dest);path=dest/'libleaf.dylib';path.write_bytes(path.read_bytes().replace(b'_puts\0',b'_pats\0'))
            p=call('adapter-run',dest/'multi');self.assertEqual(p.returncode,125);self.assertIn('_pats',p.stderr);self.assertEqual(p.stdout,'')
    def test_dependency_bad_callback_prevents_all_initializers(self):
        with tempfile.TemporaryDirectory() as temp:
            dest=Path(temp)/'images';shutil.copytree(FIX/'dylibs/classic',dest);d=self.inspect('dylibs/classic/libleaf.dylib');section=next(s for s in d['sections'] if s['flags']&255==9);data_segment=next(s for s in d['segments'] if s['name']=='__DATA');path=dest/'libleaf.dylib';data=bytearray(path.read_bytes());struct.pack_into('<Q',data,section['offset'],data_segment['vmaddr']);path.write_bytes(data)
            p=call('adapter-run',dest/'multi');self.assertEqual(p.returncode,125);self.assertIn('outside executable segments',p.stderr);self.assertEqual(p.stdout,'')
    def test_wrong_architecture_dependency(self):
        with tempfile.TemporaryDirectory() as temp:
            temp=Path(temp);shutil.copyfile(FIX/'local',temp/'local');data=bytearray((FIX/'libhelper.dylib').read_bytes());struct.pack_into('<I',data,4,0x0100000c);struct.pack_into('<I',data,8,0);(temp/'libhelper.dylib').write_bytes(data)
            p=call('adapter-run',temp/'local');self.assertEqual(p.returncode,125);self.assertIn('unsupported architecture',p.stderr)
    def test_bad_tlv_offset_rejected_before_execution(self):
        d=self.inspect('tls');section=next(s for s in d['sections'] if s['flags']&255==0x13);path=self.mutate('tls',lambda data:struct.pack_into('<Q',data,section['offset']+16,0xffffffffffffffff))
        p=call('adapter-run',path);self.assertEqual(p.returncode,125);self.assertIn('TLV offset exceeds template',p.stderr);self.assertEqual(p.stdout,'')
    def test_missing_library(self):
        path=self.mutate("hello",lambda d:d.__setitem__(slice(d.index(b"/usr/lib/libSystem.B.dylib"),d.index(b"/usr/lib/libSystem.B.dylib")+len(b"/usr/lib/libSystem.B.dylib")),b"/usr/lib/libAbsent.B.dylib"))
        p=call("adapter-run","--check",path);self.assertEqual(p.returncode,125);self.assertIn("required dependencies unresolved",p.stderr)
    def test_objc_evidence_and_rejection(self):
        d=self.inspect("objc");self.assertTrue(any("Objective-C metadata" in x for x in d["unsupported"]));p=call("adapter-run","--check",FIX/"objc");self.assertEqual(p.returncode,125)
    def test_timeout(self):
        p=call("adapter-run","--timeout","1",FIX/"loop",timeout=5);self.assertEqual(p.returncode,125);self.assertIn("timed out",p.stderr)
    def test_crash_is_in_worker(self):
        p=call("adapter-run",FIX/"crash");self.assertNotEqual(p.returncode,0)
    def test_repeated_execution(self):
        for _ in range(8):
            p=call("adapter-run",FIX/"hello","--","hello");self.assertEqual(p.returncode,0,p.stderr)
    def test_unknown_config_key(self):
        with tempfile.TemporaryDirectory() as temp:
            cfg=Path(temp)/"bad.json";cfg.write_text('{"typo":true}')
            p=call("adapter-run","--config",cfg,"--check",FIX/"hello");self.assertEqual(p.returncode,125);self.assertIn("unknown field",p.stderr)
    def test_bad_load_command(self):
        path=self.mutate("hello",lambda d:struct.pack_into("<I",d,36,0));p=call("adapter-inspect",path);self.assertNotEqual(p.returncode,0);self.assertIn("invalid load command",p.stderr)
    def test_truncated(self):
        path=self.mutate("hello",lambda d:d.__delitem__(slice(40,None)));self.assertNotEqual(call("adapter-inspect",path).returncode,0)
    def test_overlapping_universal_slices(self):
        path=self.mutate("universal",lambda d:struct.pack_into(">I",d,36,4096));p=call("adapter-inspect",path);self.assertNotEqual(p.returncode,0)
    def test_entry_must_be_executable(self):
        d=self.inspect("hello");cmd=next(c for c in d["commands"] if c["name"]=="LC_MAIN");path=self.mutate("hello",lambda data:struct.pack_into("<Q",data,cmd["offset"]+8,2**63));p=call("adapter-run",path);self.assertEqual(p.returncode,125);self.assertIn("entry is not",p.stderr)
    def test_bad_fixup(self):
        d=self.inspect("hello");off=d["dyld"]["rebase"]["offset"];path=self.mutate("hello",lambda data:data.__setitem__(off,0xff));p=call("adapter-run","--check",path);self.assertEqual(p.returncode,125);self.assertIn("unsupported rebase opcode",p.stderr)
    def test_published_macos_binary_if_fetched(self):
        if not (FIX/"reference/jq-macos-amd64").exists():self.skipTest("optional published artifact not fetched")
        d=self.inspect("reference/jq-macos-amd64");self.assertEqual(d["architecture"],"x86_64");self.assertTrue(any(s['flags']&255==0x13 for s in d['sections']));p=call("adapter-run","--check",FIX/"reference/jq-macos-amd64");self.assertEqual(p.returncode,0,p.stderr)

if __name__=="__main__":unittest.main(verbosity=2)
