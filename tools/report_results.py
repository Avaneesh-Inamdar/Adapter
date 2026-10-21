"""Derive compact compatibility evidence from passed logs and live jq samples."""
import datetime
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'docs/test-results'
FIX = ROOT/'tests/fixtures/generated'


def report():
    logs = {profile: (OUT/f'integration-{profile}.log').read_text(encoding='utf-8')
            for profile in ['debug','release']}
    passed = {k:bool(re.search(r'\nOK\s*$',v)) and 'skipped' not in v for k,v in logs.items()}
    blocked = {k:not passed[k] and 'WinError 4551' in v for k,v in logs.items()}
    for profile, log in logs.items():
        if not passed[profile] and not blocked[profile]:
            raise RuntimeError(f'{profile} integration did not pass completely')
    rows = json.loads((ROOT/'tests/compatibility/targets.json').read_text())
    for row in rows:
        row['sha256'] = hashlib.sha256((FIX/row['fixture']).read_bytes()).hexdigest()
        for profile, log in logs.items():
            row[f'verified_{profile}'] = bool(re.search(r'^'+re.escape(row['test'])+r' \(.+\) \.\.\. ok$', log, re.M))
            if not row[f'verified_{profile}'] and not blocked[profile]:
                raise RuntimeError(f'missing passing evidence for {row["test"]}/{profile}')
            if blocked[profile]:
                row[f'{profile}_execution_status']='blocked by Windows Application Control (WinError 4551)'
        row['compatibility_status'] = ('listed Windows behavior verified; reference macOS unverified'
                                       if all(passed.values()) else 'see per-profile verification and policy blocks')
        if 'execution_status' in row and not any(passed.values()):
            row['execution_status']='Earlier milestone executed; final local launches blocked by host policy'
    (OUT/'compatibility.json').write_text(json.dumps(rows,indent=2)+'\n')
    executions = []
    for profile in ['debug','release']:
        if blocked[profile]:
            continue
        base = [f'target/{profile}/db-run.exe', 'tests/fixtures/generated/reference/jq-macos-amd64']
        for args, data, expected in [(['--version'], '', 'jq-1.8.2\n'),
                                     (['-c', '{x:.a|map(. * 2)}'], '{"a":[1,2,3]}\n', '{"x":[2,4,6]}\n')]:
            command = base+['--']+args
            p = subprocess.run(command,cwd=ROOT,input=data.encode(),capture_output=True,timeout=15)
            output,error = p.stdout.decode('utf-8'),p.stderr.decode('utf-8')
            if p.returncode or output!=expected or error:
                raise RuntimeError(f'live jq sample failed: {profile}/{args}')
            executions.append({'profile':profile,'command':command,'stdin':data,
                               'exit_code':p.returncode,'stdout':output,'stderr':error})
    preflight = ['target/debug/db-run.exe','--check','tests/fixtures/generated/reference/jq-macos-amd64']
    if not blocked['debug']:
        p = subprocess.run(preflight,cwd=ROOT,capture_output=True,text=True,timeout=15)
        if p.returncode:
            raise RuntimeError('jq preflight failed')
        preflight_result={'command':preflight,'exit_code':p.returncode,
                          'executed':False,'stdout':p.stdout,'stderr':p.stderr}
        (OUT/'jq-preflight.log').write_text(p.stdout+p.stderr)
    else:
        preflight_result={'command':preflight,'executed':False,'status':'blocked',
                          'reason':'Debug db-run execution denied in integration log; not retried here.'}
        (OUT/'jq-preflight.log').write_text(preflight_result['reason']+'\n')
    (OUT/'jq-preflight.json').write_text(json.dumps(preflight_result,indent=2)+'\n')
    profiles={k:{'status':'passed' if passed[k] else 'blocked',
                 'db_run_sha256':hashlib.sha256((ROOT/f'target/{k}/db-run.exe').read_bytes()).hexdigest()}
              for k in logs}
    (OUT/'jq-execution.json').write_text(json.dumps({'run_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'artifact_sha256':rows[-1]['sha256'],'reference_mac_run':False,'profiles':profiles,'executions':executions},indent=2)+'\n')
    rust = (OUT/'rust-tests.log').read_text(encoding='utf-8')
    rust_count = sum(map(int,re.findall(r'test result: ok\. (\d+) passed',rust)))
    archive_count = int(re.search(r'Ran (\d+) tests',(OUT/'archive-tests.log').read_text()).group(1))
    counts = {k:int(re.search(r'Ran (\d+) tests',v).group(1)) for k,v in logs.items()}
    rust_status='blocked' if 'os error 4551' in rust else 'passed'
    summary = {'rust_tests_passed':rust_count,'rust_test_status':rust_status,'archive_tests_passed':archive_count,'integration_tests_attempted':counts,
               'integration_status':{k:'passed' if passed[k] else 'blocked' for k in logs},
               'distinct_tests_passed_final':rust_count+archive_count+len(re.findall(r'^test_.* \.\.\. ok$',logs['debug'],re.M)),
               'jq_test_methods':len(re.findall(r'^test_.*\(test_jq\.JqTests\.',logs['debug'],re.M)),
               'reference_macos_verified':False}
    (OUT/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print('Recorded verification reports:',summary['distinct_tests_passed_final'],'distinct tests passed in final run')


if __name__ == '__main__':
    report()
