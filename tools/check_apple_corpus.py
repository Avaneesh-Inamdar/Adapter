"""Reinspect the hash-identified, already-extracted Apple corpus without execution."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time
ROOT=Path(__file__).resolve().parents[1]
def check():
    items=json.loads((ROOT/'docs/reverse-engineering/apple-inputs/targets.json').read_text())
    results=[]
    for number,item in enumerate(items,1):
        path=ROOT/'target/artifacts'/item['source'];start=time.monotonic();digest=hashlib.sha256()
        with path.open('rb') as f:
            for block in iter(lambda:f.read(8*1024*1024),b''):digest.update(block)
        if digest.hexdigest()!=item['sha256']:raise ValueError('Apple artifact changed: '+str(path))
        p=subprocess.run([str(ROOT/'target/debug/adapter-inspect.exe'),'--json',str(path)],capture_output=True,text=True,timeout=45)
        record={'source':item['source'],'sha256':item['sha256'],'exit_code':p.returncode,'seconds':round(time.monotonic()-start,3)}
        if p.returncode:record['error']=p.stderr.strip()
        else:
            image=json.loads(p.stdout);record.update(architecture=image['architecture'],exports=len(image['exports']),dependencies=len(image['dependencies']),executed=False)
        results.append(record)
        if number%10==0:print('Apple corpus:',number,'/',len(items),flush=True)
    report={'command':[sys.executable,'tools/check_apple_corpus.py'],'all_passed':all(r['exit_code']==0 for r in results),'artifacts':results}
    (ROOT/'docs/test-results/apple-corpus-regression.json').write_text(json.dumps(report,indent=2)+'\n')
    print('Apple corpus result:',report['all_passed'],len(results),flush=True)
    return 0 if report['all_passed'] else 1
if __name__=='__main__':raise SystemExit(check())
