# Copyright (C) 2026 Toit contributors.
"""Audit complete frozen pixelation outputs without changing methods or scores."""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path

from evaluate_common_language import truth_rank


def load(path):return json.loads(Path(path).read_text())
def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def verify(files):
    for path,sha in files.items():
        if digest(path)!=sha:raise ValueError('Frozen pixelation input changed:'+path)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan);root=Path(plan['experiment_root']);evaluation=root/'evaluation'
    freeze=load(evaluation/'freeze.json');verify(freeze['files']);summary_path=evaluation/'summary.json';summary=load(summary_path)
    if not summary['all_frozen_hashes_verified'] or len(summary['rows'])!=plan['cases']:raise ValueError('Complete pixelation report required')
    cases={};artifacts=[a.plan,Path(__file__),evaluation/'freeze.json',summary_path]
    for condition in ['pixelate-8','pixelate-16']:
        for availability in ['present','absent']:
            path=Path(plan['benchmark'])/f'test-{condition}-{availability}.json';artifacts.append(path)
            for case in load(path)['cases']:cases[case['id']]=(condition,availability,case)
    if len(cases)!=plan['cases'] or {r['case'] for r in summary['rows']}!=cases.keys():raise ValueError('Planned pixelation coverage differs')
    checked=Counter();statuses=Counter()
    for row in summary['rows']:
        condition,availability,case=cases[row['case']]
        if (row['condition'],row['availability'])!=(condition,availability):raise ValueError('Pixelation case metadata differs')
        allowed={c['id'] for c in case['input']['candidates']}
        truth=case['truth_id'] if availability=='present' else None
        if (case['truth_id'] in allowed)!=(availability=='present'):raise ValueError('Availability label differs')
        if set(row['engines'])!={'legacy','unit','shared_cells'}:raise ValueError('Missing planned engine')
        for engine,value in row['engines'].items():
            statuses[engine,value['status']]+=1
            if value['status']!='ok':continue
            path=evaluation/row['case']/f'{engine}.json';report=load(path);artifacts.append(path)
            if engine=='legacy':
                items=report['candidates'];scores={c['id']:-c['evidence_score'] for c in items}
                if report['tested_candidates']!=len(allowed) or not scores.keys()<=allowed:raise ValueError('Legacy candidate enumeration differs')
                for c in items:
                    if [c['evidence_rank_min'],c['evidence_rank_max']]!=truth_rank(scores,c['id']):
                        raise ValueError('Legacy actual rank interval differs from cost replay')
            else:
                items=report['scores'];scores={c['id']:-c['cost'] for c in items if c['cost'] is not None}
                if {c['id'] for c in items}!=allowed or len(items)!=len(allowed):raise ValueError('Pixelation output lost supplied candidates')
                order=[c['id'] for c in items]
                expected=[c['id'] for c in sorted(items,key=lambda c:(c['cost'] if c['cost'] is not None else math.inf,c['id']))]
                if order!=expected:raise ValueError('Actual pixelation score list is incorrectly ordered')
            if any(not math.isfinite(s) or s>0 for s in scores.values()):raise ValueError('Invalid pixelation cost')
            if len(scores)!=value['renderable_candidates']:raise ValueError('Renderable count differs')
            if truth_rank(scores,truth)!=value['truth_rank']:raise ValueError('Actual output rank differs from evaluator grade')
            checked[engine]+=1
    verify(freeze['files'])
    result=dict(planned_cases=len(cases),successful_reports_checked=dict(checked),
        statuses=[dict(engine=k[0],status=k[1],count=v) for k,v in sorted(statuses.items())],
        source_files={str(p.resolve()):digest(p) for p in artifacts},
        interpretation='Complete frozen input hashes reverified. Every successful actual CLI report checked for supplied candidate coverage, finite costs, displayed score order or native rank intervals, renderable counts and evaluator truth ranks. Failures remain in planned denominators. No candidate score, selected method or raw report changed.')
    with a.output.open('x') as f:json.dump(result,f,indent=2);f.write('\n')
    print(json.dumps({k:v for k,v in result.items() if k not in ['source_files','interpretation']}))


if __name__=='__main__':main()
