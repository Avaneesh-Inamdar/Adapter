import bz2
import hashlib
import io
import json
import lzma
from pathlib import Path
import plistlib
import struct
import sys
import tempfile
import unittest
import zlib
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'tools'))
from inspect_xip import PBZX,xar,inspect,exact
from read_xip_member import read_member

def pbzx(data,compressed=True):
    block=lzma.compress(data) if compressed else data
    return b'pbzx'+struct.pack('>QQQ',0x1000000,len(data),len(block))+block

def cpio(name,data=b'',mode=0o100644):
    name=name.encode()+b'\0'
    return (f'070707{0:06o}{0:06o}{mode:06o}{0:06o}{0:06o}{1:06o}{0:06o}{0:011o}{len(name):06o}{len(data):011o}'.encode()+name+data)

def xip(content):
    metadata=bz2.compress(plistlib.dumps({'UncompressedSize':len(content)}))
    payload=pbzx(content)
    toc='<xar><toc>'
    for name,off,data,encoding in [('Content',0,payload,'application/octet-stream'),('Metadata',len(payload),metadata,'application/x-bzip2')]:
        toc+=f'<file><name>{name}</name><data><offset>{off}</offset><length>{len(data)}</length><size>{len(data)}</size><encoding style="{encoding}"/><archived-checksum>{hashlib.sha1(data).hexdigest()}</archived-checksum></data></file>'
    toc+='</toc></xar>';raw=toc.encode();compressed=zlib.compress(raw)
    return struct.pack('>4sHHQQI',b'xar!',28,1,len(compressed),len(raw),1)+compressed+payload+metadata

class ArchiveTests(unittest.TestCase):
    def test_compressed_and_plain_pbzx(self):
        for compress in [True,False]:
            data=pbzx(b'archive bytes',compress);reader=PBZX(io.BytesIO(data),len(data));self.assertEqual(reader.read(100),b'archive bytes');self.assertEqual(reader.read(1),b'');self.assertEqual(reader.digest.hexdigest(),hashlib.sha1(data).hexdigest())
    def test_pbzx_block_length_mismatch(self):
        data=bytearray(pbzx(b'data'));struct.pack_into('>Q',data,12,3)
        with self.assertRaises(ValueError):PBZX(io.BytesIO(data),len(data)).read(100)
    def test_truncated_xar(self):
        with self.assertRaises(ValueError):xar(io.BytesIO(b'xar!'))
    def test_end_to_end_selective_extraction_and_integrity(self):
        content=cpio('./Xcode.app/Contents/MacOS/Xcode',b'test bytes')+cpio('TRAILER!!!')
        with tempfile.TemporaryDirectory() as temp:
            src=Path(temp)/'test.xip';src.write_bytes(xip(content));out=Path(temp)/'analysis';inspect(src,out)
            self.assertEqual((out/'selected/Xcode.app/Contents/MacOS/Xcode').read_bytes(),b'test bytes');self.assertTrue(json.loads((out/'summary.json').read_text())['content_sha1_verified'])
    def test_traversal_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            src=Path(temp)/'bad.xip';src.write_bytes(xip(cpio('../escape',b'bad')+cpio('TRAILER!!!')))
            with self.assertRaisesRegex(ValueError,'unsafe'):inspect(src,Path(temp)/'analysis')
            self.assertFalse((Path(temp)/'escape').exists())
    def test_existing_output_is_preserved(self):
        with tempfile.TemporaryDirectory() as temp:
            out=Path(temp)/'analysis';out.mkdir()
            with self.assertRaisesRegex(ValueError,'new'):inspect(Path(temp)/'missing',out)
    def test_random_access_member_is_bound_to_source_and_header(self):
        name='Xcode.app/Contents/MacOS/Xcode'
        content=cpio('.',mode=0o40755)+cpio('./other',b'other bytes')+cpio('./'+name,b'program bytes')+cpio('TRAILER!!!')
        with tempfile.TemporaryDirectory() as temp:
            src=Path(temp)/'test.xip';src.write_bytes(xip(content));out=Path(temp)/'analysis';inspect(src,out)
            target=Path(temp)/'followup/program';read_member(src,out,name,target)
            self.assertEqual(target.read_bytes(),b'program bytes')
            src.write_bytes(src.read_bytes()+b'changed')
            with self.assertRaisesRegex(ValueError,'SHA-256'):read_member(src,out,name,Path(temp)/'second')
    def test_random_access_member_rejects_wrong_inventory_offset(self):
        name='Xcode.app/Contents/MacOS/Xcode'
        content=cpio('./other',b'other bytes')+cpio('./'+name,b'program bytes')+cpio('TRAILER!!!')
        with tempfile.TemporaryDirectory() as temp:
            src=Path(temp)/'test.xip';src.write_bytes(xip(content));out=Path(temp)/'analysis';inspect(src,out)
            inventory=out/'inventory.jsonl';items=[json.loads(line) for line in inventory.read_text().splitlines()];items[0]['size']+=1;inventory.write_text(''.join(json.dumps(e)+'\n' for e in items))
            with self.assertRaisesRegex(ValueError,'header'):read_member(src,out,name,Path(temp)/'program')
    def test_raw_filesystem_placeholder_is_reported(self):
        name='Xcode.app/Contents/MacOS/Xcode';content=cpio('./'+name,b'NULLcanary'+b'\0'*32)+cpio('TRAILER!!!')
        with tempfile.TemporaryDirectory() as temp:
            src=Path(temp)/'test.xip';src.write_bytes(xip(content));out=Path(temp)/'analysis';inspect(src,out);target=Path(temp)/'raw';read_member(src,out,name,target)
            report=json.loads(target.with_suffix('.provenance.json').read_text());self.assertTrue(report['filesystem_placeholder']);self.assertTrue(report['raw_data_fork_only'])
if __name__=='__main__':unittest.main(verbosity=2)
