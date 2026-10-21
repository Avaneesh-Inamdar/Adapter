"""Persist observed Mach-O and plist facts from selectively extracted inputs."""
import hashlib
import json
from pathlib import Path
import plistlib
import subprocess

ROOT=Path(__file__).resolve().parents[1]
ARTIFACTS=ROOT/'target/artifacts'
OUT=ROOT/'docs/reverse-engineering/apple-inputs'
RAW=ARTIFACTS/'raw-inspection'
MAGIC={b'\xcf\xfa\xed\xfe',b'\xca\xfe\xba\xbe',b'\xca\xfe\xba\xbf'}

def analyze():
    OUT.mkdir(parents=True,exist_ok=True)
    RAW.mkdir(parents=True,exist_ok=True)
    roots=[ARTIFACTS/'xcode27-inspection/selected',ARTIFACTS/'additional-tools27',ARTIFACTS/'icon-composer27',ARTIFACTS/'clt27-inspection/selected',ARTIFACTS/'xcode27-followup']
    summary=[];bundles=[]
    for root in roots:
        if not root.exists():continue
        for p in sorted(root.rglob('*')):
            if not p.is_file():continue
            relative=str(p.relative_to(ARTIFACTS)).replace('\\','/')
            if p.name=='Info.plist':
                d=plistlib.loads(p.read_bytes());keys=['CFBundleIdentifier','CFBundleExecutable','CFBundleName','CFBundleShortVersionString','CFBundleVersion','LSMinimumSystemVersion','DTXcode','DTXcodeBuild']
                bundles.append({'source':relative,**{k:d.get(k) for k in keys}})
            with p.open('rb') as f:magic=f.read(4)
            if magic not in MAGIC:continue
            ident=hashlib.sha256(relative.encode()).hexdigest()[:16]
            proc=subprocess.run([str(ROOT/'target/debug/adapter-inspect.exe'),'--json',str(p)],capture_output=True,text=True,timeout=30)
            if proc.returncode:
                summary.append({'source':relative,'error':proc.stderr.strip()});continue
            image=json.loads(proc.stdout)
            (RAW/(ident+'.json')).write_text(proc.stdout)
            compact=dict(image)
            compact['total_symbol_count']=len(image['symbols'])
            compact['symbols']=[s for s in image['symbols'] if s['undefined'] or s['name'] in ['_main','_IDEApplicationLoad']]
            compact['symbol_coverage']='All undefined symbols plus selected entry symbols; full symbol table in local target/artifacts/raw-inspection.'
            (OUT/(ident+'.json')).write_text(json.dumps(compact,indent=2)+'\n')
            digest=hashlib.sha256()
            with p.open('rb') as f:
                for b in iter(lambda:f.read(8*1024*1024),b''):digest.update(b)
            imports=[s['name'] for s in image['symbols'] if s['undefined']]
            summary.append({'source':relative,'name':p.name,'size':p.stat().st_size,'sha256':digest.hexdigest(),'architecture':image['architecture'],'slice_cpus':[hex(s['cpu']) for s in image['slices']],'filetype':image['filetype'],'entry_file_offset':image['entry_file_offset'],'dependencies':[d['name'] for d in image['dependencies']],'imports':imports,'chained_fixups':image['chained_fixups'],'unsupported':image['unsupported'],'inspection_file':ident+'.json','executed':False})
    (OUT/'targets.json').write_text(json.dumps(summary,indent=2)+'\n')
    (OUT/'bundles.json').write_text(json.dumps(bundles,indent=2)+'\n')
    print('Inspected',len(summary),'Mach-O images;',len(bundles),'bundle plists')
    for s in summary:print(s.get('name',s['source']),s.get('architecture',s.get('error')),len(s.get('imports',[])),'imports')
if __name__=='__main__':analyze()
