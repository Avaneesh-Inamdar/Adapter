"""Read selected XAR package members and PBZX CPIO tools, checking payload SHA-1."""
import argparse
import hashlib
import json
from pathlib import Path,PurePosixPath
import stat
import struct
import time
import xml.etree.ElementTree as ET
import zlib
from inspect_xip import PBZX,exact,LIMIT

NAMES={'ar','clang','clang++','ld','lipo','nm','otool','size','strings','strip','xcrun','xcodebuild','swift','swiftc','llvm-nm','llvm-otool','size-classic','install_name_tool','dsymutil','dwarfdump','codesign_allocate','ctags'}

def inspect(path,out):
    if out.exists():raise ValueError('output must be new')
    out.mkdir(parents=True);started=time.monotonic()
    with path.open('rb') as f:
        magic,hs,version,cl,ul,checksum=struct.unpack('>4sHHQQI',exact(f,28))
        if magic!=b'xar!' or cl>LIMIT or ul>LIMIT:raise ValueError('unsupported XAR')
        f.seek(hs);root=ET.fromstring(zlib.decompress(exact(f,cl)))
        pkg=next(e for e in root.findall('./toc/file') if e.findtext('name')=='CLTools_Executables.pkg')
        entries={e.findtext('name'):e for e in pkg.findall('file')}
        def member(name):
            e=entries[name];f.seek(hs+cl+int(e.findtext('data/offset')));length=int(e.findtext('data/length'))
            return e,length
        metadata={}
        for name in ['PackageInfo','Bom']:
            if name not in entries:continue
            e,n=member(name);data=exact(f,n);expected=e.findtext('data/archived-checksum')
            if expected and hashlib.sha1(data).hexdigest()!=expected:raise ValueError(f'{name} SHA-1 mismatch')
            encoding=e.find('data/encoding').get('style')
            if encoding=='application/x-bzip2':
                import bz2
                data=bz2.decompress(data)
            elif encoding=='application/zlib':data=zlib.decompress(data)
            (out/name).write_bytes(data);metadata[name]={'size':len(data),'encoding':encoding}
        e,n=member('Payload');expected=e.findtext('data/archived-checksum');reader=PBZX(f,n)
        selected=[];count=0
        with (out/'inventory.jsonl').open('w',encoding='utf-8') as inventory:
            while True:
                header=exact(reader,76)
                if header[:6]!=b'070707':raise ValueError('unsupported CPIO')
                mode=int(header[18:24],8);namesize=int(header[59:65],8);size=int(header[65:76],8)
                if not 0<namesize<=16384:raise ValueError('invalid CPIO name')
                name=exact(reader,namesize)[:-1].decode('utf-8','surrogateescape')
                if name=='TRAILER!!!':break
                pp=PurePosixPath(name)
                if pp.is_absolute() or '..' in pp.parts or '\\' in name:raise ValueError('unsafe CPIO path')
                record={'path':str(pp),'size':size,'mode':oct(mode),'kind':'file' if stat.S_ISREG(mode) else 'symlink' if stat.S_ISLNK(mode) else 'other'}
                if stat.S_ISREG(mode) and pp.name in NAMES and 'bin' in pp.parts and size<128*1024*1024:
                    target=out/'selected'/Path(*pp.parts);target.parent.mkdir(parents=True,exist_ok=True);sha=hashlib.sha256()
                    with target.open('xb') as dest:
                        left=size
                        while left:
                            block=exact(reader,min(left,1024*1024));dest.write(block);sha.update(block);left-=len(block)
                    record['extracted_sha256']=sha.hexdigest();selected.append(record.copy());print('Selected:',record['path'],size,flush=True)
                elif stat.S_ISLNK(mode) and size<16384:record['link_target']=exact(reader,size).decode('utf-8','replace')
                else:reader.skip(size)
                inventory.write(json.dumps(record)+'\n');count+=1
            while reader.read(1024*1024):pass
        if reader.digest.hexdigest()!=expected:raise ValueError('Payload archived SHA-1 mismatch')
        result={'source':str(path.resolve()),'payload_sha1':expected,'payload_sha1_verified':True,'apple_signature_verified':False,'entries':count,'decoded_bytes':reader.decoded,'seconds':round(time.monotonic()-started,2),'package_metadata':metadata,'selected':selected}
        (out/'summary.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k!='selected'},indent=2),flush=True)
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('input',type=Path);p.add_argument('output',type=Path);a=p.parse_args();inspect(a.input,a.output)
