"""Bounded XAR/PBZX/odc inventory and selective extraction for local research.

Does not execute the payload, create archive symlinks, or validate Apple signing.
All selected files are copied under a new analysis directory. Original stays read-only.
"""
import argparse
import bz2
import hashlib
import json
import lzma
from pathlib import Path, PurePosixPath
import plistlib
import stat
import struct
import time
import xml.etree.ElementTree as ET
import zlib

LIMIT=32*1024*1024

def exact(f,n):
    d=f.read(n)
    if len(d)!=n:raise ValueError(f"truncated stream: need {n}, got {len(d)}")
    return d

def xar(f):
    magic,header,version,compressed,uncompressed,checksum=struct.unpack('>4sHHQQI',exact(f,28))
    if magic!=b'xar!' or version!=1 or compressed>LIMIT or uncompressed>LIMIT:raise ValueError("unsupported XAR header")
    f.seek(header)
    decoder=zlib.decompressobj();toc=decoder.decompress(exact(f,compressed),uncompressed+1)
    if len(toc)!=uncompressed or not decoder.eof:raise ValueError("XAR TOC size mismatch")
    root=ET.fromstring(toc)
    files=[]
    for e in root.findall('./toc/file'):
        files.append({"name":e.findtext('name'),"offset":header+compressed+int(e.findtext('data/offset')),"length":int(e.findtext('data/length')),"size":int(e.findtext('data/size')),"encoding":e.find('data/encoding').get('style'),"archived_sha1":e.findtext('data/archived-checksum')})
    return files

class PBZX:
    def __init__(self,f,length):
        self.f=f;self.remaining=length;self.buffer=b'';self.position=0;self.blocks=0;self.decoded=0;self.digest=hashlib.sha1();self.last=time.monotonic()
        if self.input(4)!=b'pbzx':raise ValueError("missing PBZX header")
        self.chunk_size=int.from_bytes(self.input(8),'big')
        if self.chunk_size>LIMIT:raise ValueError("PBZX chunk size exceeds limit")
    def input(self,n):
        if n>self.remaining:raise ValueError("PBZX read exceeds XAR member")
        d=exact(self.f,n);self.remaining-=n;self.digest.update(d);return d
    def chunk(self):
        if not self.remaining:return False
        raw_size=int.from_bytes(self.input(8),'big');compressed=int.from_bytes(self.input(8),'big')
        if raw_size>LIMIT or compressed>LIMIT:raise ValueError("PBZX block exceeds limit")
        block=self.input(compressed)
        if block.startswith(b'\xfd7zXZ\x00'):
            decoder=lzma.LZMADecompressor(memlimit=256*1024*1024);plain=decoder.decompress(block,max_length=raw_size+1)
            if not decoder.eof or decoder.unused_data:raise ValueError("incomplete or trailing XZ block")
        else:plain=block
        if len(plain)!=raw_size:raise ValueError("PBZX decoded block size mismatch")
        self.buffer=plain;self.position=0;self.blocks+=1;self.decoded+=len(plain)
        if time.monotonic()-self.last>15:
            print(f"PBZX: {self.blocks} blocks, {self.decoded/1024**3:.2f} GiB decoded, {self.remaining/1024**2:.0f} MiB input remaining",flush=True);self.last=time.monotonic()
        return True
    def read(self,n):
        output=[]
        while n:
            if self.position==len(self.buffer) and not self.chunk():break
            take=min(n,len(self.buffer)-self.position);output.append(self.buffer[self.position:self.position+take]);self.position+=take;n-=take
        return b''.join(output)
    def skip(self,n):
        while n:
            if self.position==len(self.buffer) and not self.chunk():raise ValueError("truncated CPIO data")
            take=min(n,len(self.buffer)-self.position);self.position+=take;n-=take

def selected(path,size):
    if size>128*1024*1024:return False
    wanted={
        'Xcode.app/Contents/Info.plist','Xcode.app/Contents/version.plist','Xcode.app/Contents/MacOS/Xcode',
        'Xcode.app/Contents/Developer/usr/bin/xcodebuild','Xcode.app/Contents/Developer/usr/bin/xcrun',
        'Xcode.app/Contents/SharedFrameworks/DVTFoundation.framework/Versions/A/DVTFoundation',
        'Xcode.app/Contents/SharedFrameworks/IDEFoundation.framework/Versions/A/IDEFoundation',
    }
    if path in wanted:return True
    prefix='Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/'
    return path.startswith(prefix) and path[len(prefix):] in {'clang','ld','ar','nm','otool','lipo','size','strings','strip','dwarfdump','dsymutil','swiftc','swift-frontend'}

def inspect(path,out):
    if out.exists():raise ValueError(f"output must be new: {out}")
    out.mkdir(parents=True)
    started=time.monotonic();extracted=[];count=0;total=0
    with path.open('rb') as f:
        members=xar(f);(out/'xar-members.json').write_text(json.dumps(members,indent=2)+'\n')
        content=next(e for e in members if e['name']=='Content')
        metadata=next(e for e in members if e['name']=='Metadata');f.seek(metadata['offset']);raw=bz2.decompress(exact(f,metadata['length']))
        (out/'Metadata.plist').write_bytes(raw);meta=plistlib.loads(raw)
        f.seek(content['offset']);reader=PBZX(f,content['length'])
        with (out/'inventory.jsonl').open('w',encoding='utf-8') as inventory:
            while True:
                header=exact(reader,76)
                if header[:6]!=b'070707':raise ValueError(f"unsupported CPIO header {header[:16]!r}")
                mode=int(header[18:24],8);namesize=int(header[59:65],8);size=int(header[65:76],8)
                if namesize==0 or namesize>16384:raise ValueError("invalid CPIO name size")
                name=exact(reader,namesize)
                if name[-1:]!=b'\0':raise ValueError("CPIO filename has no terminator")
                name=name[:-1].decode('utf-8','surrogateescape')
                if name=='TRAILER!!!':break
                pp=PurePosixPath(name)
                if pp.is_absolute() or '..' in pp.parts or '\\' in name:raise ValueError(f"unsafe archive name {name!r}")
                logical=str(pp);record={"path":logical,"size":size,"mode":oct(mode),"kind":"file" if stat.S_ISREG(mode) else "symlink" if stat.S_ISLNK(mode) else "directory" if stat.S_ISDIR(mode) else "other"}
                if stat.S_ISREG(mode) and selected(logical,size):
                    if ':' in logical:raise ValueError("selected path cannot be materialized on Windows")
                    target=out/'selected'/Path(*pp.parts);target.parent.mkdir(parents=True,exist_ok=True)
                    digest=hashlib.sha256()
                    with target.open('xb') as dest:
                        left=size
                        while left:
                            block=exact(reader,min(left,1024*1024));dest.write(block);digest.update(block);left-=len(block)
                    record['extracted_sha256']=digest.hexdigest();extracted.append(record.copy());print(f"Selected: {logical} ({size} bytes)",flush=True)
                elif stat.S_ISLNK(mode) and size<16384:record['link_target']=exact(reader,size).decode('utf-8','replace')
                else:reader.skip(size)
                inventory.write(json.dumps(record,ensure_ascii=True)+'\n');count+=1;total+=size
            # Consume trailing padding, preserving the full Content member hash.
            while reader.read(1024*1024):pass
        if reader.digest.hexdigest()!=content['archived_sha1']:raise ValueError("XAR Content SHA-1 mismatch")
    sha=hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda:f.read(8*1024*1024),b''):sha.update(block)
    result={"source":str(path.resolve()),"source_size":path.stat().st_size,"source_sha256":sha.hexdigest(),"content_sha1_verified":True,"apple_signature_verified":False,"metadata":meta,"entries":count,"file_data_bytes":total,"decoded_bytes":reader.decoded,"pbzx_blocks":reader.blocks,"seconds":round(time.monotonic()-started,2),"selected":extracted}
    (out/'summary.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k!='selected'},indent=2),flush=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('input',type=Path);parser.add_argument('output',type=Path);args=parser.parse_args();inspect(args.input,args.output)
