"""Read one member from an already verified XIP inventory without full expansion.

The observed archive uses '.' and './path' odc names. Reconstruct the byte
position from its complete inventory, then validate the actual CPIO header/name.
Only intersecting PBZX blocks are decompressed. Source SHA-256 binds inventory.
"""
import argparse
import hashlib
import json
import lzma
from pathlib import Path,PurePosixPath
import stat
from inspect_xip import exact,xar,LIMIT

def read_member(source,analysis,name,output):
    if output.exists():raise ValueError('output file must be new')
    pp=PurePosixPath(name)
    if pp.is_absolute() or '..' in pp.parts or '\\' in name or ':' in name:raise ValueError('unsafe member name')
    summary=json.loads((analysis/'summary.json').read_text());sha=hashlib.sha256()
    with source.open('rb') as f:
        for b in iter(lambda:f.read(8*1024*1024),b''):sha.update(b)
    if sha.hexdigest()!=summary['source_sha256']:raise ValueError('source does not match inventory SHA-256')
    pos=0;found=None
    with (analysis/'inventory.jsonl').open(encoding='utf-8') as f:
        for line in f:
            e=json.loads(line);raw_name='.' if e['path']=='.' else './'+e['path'];n=len(raw_name.encode('utf-8','surrogateescape'))+1
            if e['path']==str(pp):found=(pos,n,e);break
            pos+=76+n+e['size']
    if found is None:raise ValueError('member not in verified inventory')
    pos,n,entry=found
    if entry['kind']!='file' or entry['size']>128*1024*1024:raise ValueError('member is not a supported regular file')
    begin,end=pos,pos+76+n+entry['size'];decoded=0;parts=[]
    with source.open('rb') as f:
        content=next(e for e in xar(f) if e['name']=='Content');f.seek(content['offset'])
        if exact(f,4)!=b'pbzx':raise ValueError('missing PBZX')
        exact(f,8);remaining=content['length']-12
        while remaining:
            raw=int.from_bytes(exact(f,8),'big');packed=int.from_bytes(exact(f,8),'big');remaining-=16
            if raw>LIMIT or packed>LIMIT or packed>remaining:raise ValueError('invalid PBZX block bounds')
            if decoded<end and decoded+raw>begin:
                block=exact(f,packed)
                if block.startswith(b'\xfd7zXZ\x00'):
                    decoder=lzma.LZMADecompressor(memlimit=256*1024*1024);plain=decoder.decompress(block,max_length=raw+1)
                    if not decoder.eof or decoder.unused_data:raise ValueError('invalid XZ block')
                else:plain=block
                if len(plain)!=raw:raise ValueError('PBZX size mismatch')
                parts.append(plain[max(0,begin-decoded):min(raw,end-decoded)])
            else:f.seek(packed,1)
            remaining-=packed;decoded+=raw
            if decoded>=end:break
    data=b''.join(parts);header=data[:76]
    if header[:6]!=b'070707' or int(header[59:65],8)!=n or int(header[65:76],8)!=entry['size'] or not stat.S_ISREG(int(header[18:24],8)):raise ValueError('reconstructed CPIO header does not match inventory')
    actual=data[76:76+n]
    if actual!=('./'+str(pp)).encode()+b'\0':raise ValueError('reconstructed CPIO member name mismatch')
    payload=data[76+n:]
    if len(payload)!=entry['size']:raise ValueError('short member')
    output.parent.mkdir(parents=True,exist_ok=True);output.write_bytes(payload)
    result={'source_sha256':summary['source_sha256'],'member':str(pp),'size':len(payload),'sha256':hashlib.sha256(payload).hexdigest(),'reconstructed_cpio_offset':pos,'header_and_name_verified':True,'raw_data_fork_only':True,'filesystem_placeholder':payload.startswith(b'NULLcanary')}
    output.with_suffix(output.suffix+'.provenance.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('source',type=Path);p.add_argument('analysis',type=Path);p.add_argument('member');p.add_argument('output',type=Path);a=p.parse_args();read_member(a.source,a.analysis,a.member,a.output)
