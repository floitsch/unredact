# Copyright (C) 2026 Toit contributors.
"""Run registered native-output controls once their complete inputs exist."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


root=Path(__file__).resolve().parent.parent
workspace=root.parent.parent
os.chdir(workspace)
plan=json.loads((root/'plan.json').read_text())
geometry=Path(plan['geometry_root'])
geometry_plan=json.loads((geometry/'plan.json').read_text())
deadline=time.monotonic()+28800


def verify():
    for path,sha in json.loads((root/'freeze.json').read_text()).items():
        if hashlib.sha256(Path(path).read_bytes()).hexdigest()!=sha:
            raise ValueError(f'Registered native-output control changed:{path}')


def ready(path,count,field='executions'):
    try:report=json.loads(path.read_text())
    except (FileNotFoundError,json.JSONDecodeError):return False
    return len(report.get(field,[]))==count and (field!='rows' or report.get('all_frozen_hashes_verified'))


def wait_for(predicate):
    while not predicate():
        if time.monotonic()>deadline:raise TimeoutError('Complete native-control inputs not available after eight hours')
        time.sleep(5)
    verify()


def run(script,output):
    subprocess.run([str(workspace/'runs/baseline-venv/bin/python'),str(root/'runtime'/script),
        '--plan',str(root/'plan.json'),'--output',str(root/output)],check=True,
        env={**os.environ,'PYTHONDONTWRITEBYTECODE':'1'})


verify()
print('Waiting for complete geometry reports; no partial accuracy analysis.',flush=True)
wait_for(lambda:all(ready(geometry/d/'comparison.json',plan['spans']*len(geometry_plan['adapters']))
    for d in geometry_plan['availability_runs'].values()))
run('audit_native_morten_decisions.py','native-decisions.json')
print('Native decisions audited. Waiting for complete common-language report.',flush=True)
wait_for(lambda:ready(Path(plan['language_evaluation'])/'summary.json',2*plan['spans'],'rows'))
run('replay_morten_confidence.py','confidence-controls.json')
print('All registered native-output controls completed.',flush=True)
