"""Run and retain actual verification results; fail on the first failed stage."""
import datetime
import json
import os
from pathlib import Path
import subprocess
import sys
import time
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'docs/test-results'
def verify():
    OUT.mkdir(parents=True,exist_ok=True)
    llvm=Path(os.environ.get('ADAPTER_LLVM',r'C:\Program Files\LLVM\bin'))
    native=[str(p.relative_to(ROOT)) for p in (ROOT/'src/darwin/host').iterdir() if p.suffix in ['.c','.h']]
    native+=['tests/fixtures/libc.c','tests/fixtures/file_time.c','tests/fixtures/condvars.c','tests/fixtures/cxx_tls.cpp','tests/fixtures/plugin.c','tests/fixtures/plugin_tls.c','tests/fixtures/dlopen.c','tests/fixtures/mach_server.c','tests/fixtures/mach_client.c','tests/fixtures/mach_errors.c','tests/fixtures/xpc_server.c','tests/fixtures/xpc_client.c','tests/fixtures/xpc_missing.c']
    native+=['tests/fixtures/objc_core.m','tests/fixtures/objc_category.m','tests/fixtures/objc_metadata.m','tests/fixtures/objc_arc.m']
    stages=[
        ('format',['cargo','fmt','--all','--','--check'],None),
        ('abi-contracts',[sys.executable,'tools/check_contracts.py'],None),
        ('phase-docs',[sys.executable,'tools/update_phases.py'],None),
        ('native-format',[str(llvm/'clang-format.exe'),'--dry-run','--Werror',*native],None),
        ('clippy',['cargo','clippy','--locked','--all-targets','--','-D','warnings'],None),
        ('rust-tests',['cargo','test','--locked'],None),
        ('debug-build',['cargo','build','--locked'],None),
        ('reference-artifact',[sys.executable,'tools/fetch_jq.py'],None),
        ('reference-ripgrep',[sys.executable,'tools/fetch_ripgrep.py'],None),
        ('reference-test-data',[sys.executable,'tools/fetch_jq_tests.py'],None),
        ('fixtures',[sys.executable,'tests/build_fixtures.py'],None),
        ('archive-tests',[sys.executable,'-m','unittest','discover','-s','tests/unit','-v'],None),
        ('integration-debug',[sys.executable,'-m','unittest','discover','-s','tests/integration','-v'],None),
        ('release-build',['cargo','build','--release','--locked'],None),
        ('integration-release',[sys.executable,'-m','unittest','discover','-s','tests/integration','-v'],{'ADAPTER_BIN':str(ROOT/'target/release')}),
        ('compatibility-report',[sys.executable,'tools/report_results.py'],None),
    ]
    results=[]
    for name,cmd,extra in stages:
        start=time.monotonic();print('Running:',name,flush=True)
        proc=subprocess.run(cmd,cwd=ROOT,env={**os.environ,**(extra or {})},capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=600)
        log=proc.stdout+proc.stderr;(OUT/(name+'.log')).write_text(log,encoding='utf-8')
        result={'stage':name,'command':cmd,'exit_code':proc.returncode,'seconds':round(time.monotonic()-start,3),'log':name+'.log'}
        if proc.returncode and ('WinError 4551' in log or 'os error 4551' in log):
            result['status']='blocked'
            result['reason']='Windows Application Control prevented executable launch (WinError 4551); see CodeIntegrity events 3033/3077.'
        results.append(result)
        report={'run_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'all_passed':all(r['exit_code']==0 for r in results),'stages':results}
        if any(r.get('status')=='blocked' for r in results):report['verification_status']='blocked'
        (OUT/'latest.json').write_text(json.dumps(report,indent=2)+'\n');print(name,proc.returncode,flush=True)
        if proc.returncode:
            if result.get('status')=='blocked':
                print('Blocked by Windows Application Control; retained full log:',result['log'])
                continue  # Retain a failing/blocked run, but still produce honest reports.
            print(log[-12000:])
            return proc.returncode
    return 0 if all(r['exit_code']==0 for r in results) else 1
if __name__=='__main__':raise SystemExit(verify())
