# Copyright (C) 2026 Toit contributors.
"""Verify saved CLI ranking order, tie groups and grades against cached scores.

The evaluator already checks candidate scores. This additional audit checks the
ordering users actually receive. It never runs a model or changes a score.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path

from evaluate_heldout_language import summarize, verify
from evaluate_common_language import truth_rank


def load(path):return json.loads(Path(path).read_text())
def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def groups(scores):
    ordered=sorted(scores,key=lambda k:(-scores[k],k));result=[];i=0
    while i<len(ordered):
        j=i+1
        while j<len(ordered) and abs(scores[ordered[j]]-scores[ordered[i]])<=1e-9:j+=1
        result.append(ordered[i:j]);i=j
    return result


def check_report(report,scores,weight,truth):
    physical=report['physical'];count=physical['independent_observations']
    if count!=1:raise ValueError('This audit requires single-occurrence cases')
    costs={c['id']:c['evidence_score']/count for c in physical['candidates']}
    allowed={c['id'] for c in physical['candidates'] if c['compatible']}
    if not allowed<=scores.keys():raise ValueError('CLI filter contains unknown candidate')
    expected={k:scores[k]-weight*costs[k] for k in allowed}
    ranked=report['ranked'];actual={c['id']:c for c in ranked}
    if len(actual)!=len(ranked) or set(actual)!=allowed:raise ValueError('CLI ranked candidate coverage differs')
    actual_scores={}
    for key,c in actual.items():
        value=c['combined_score'] if weight else c['mean_sequence_log_score']
        if not math.isfinite(value) or abs(value-expected[key])>1e-9 or abs(c['mean_sequence_log_score']-scores[key])>1e-9:
            raise ValueError('CLI candidate score differs from independent replay')
        actual_scores[key]=value
    if [c['id'] for c in ranked]!=sorted(actual,key=lambda k:(-actual_scores[k],k)):
        raise ValueError('CLI ranked list order differs from reported scores')
    if report['ranked_groups']!=groups(actual_scores):raise ValueError('CLI tie groups differ from its sorted scores')
    if [set(g) for g in report['ranked_groups']]!=[set(g) for g in groups(expected)]:
        raise ValueError('CLI tie groups differ from independent score replay')
    interval=None;offset=0
    for group in report['ranked_groups']:
        if truth in group:interval=[offset+1,offset+len(group)]
        offset+=len(group)
    if interval!=truth_rank(expected,truth):raise ValueError('Actual CLI group rank differs from evaluator truth rank')
    return dict(truth_rank=interval,unique_correct_first=interval==[1,1],compatible=len(allowed))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan);verify(load(a.plan.parent/'freeze.json'))
    evaluation=Path(plan['evaluation']);freeze=load(evaluation/'freeze.json');verify(freeze['files'])
    raw=load(evaluation/'summary.json');geometry=Path(plan['geometry_root']);gp=load(geometry/'plan.json')
    if len(raw['rows'])!=2*plan['spans'] or not raw['all_frozen_hashes_verified']:raise ValueError('Complete language report required')
    rows=copy.deepcopy(raw['rows']);by_case={r['case']:r for r in rows};checked=[]
    artifacts=[a.plan,Path(__file__),evaluation/'summary.json',evaluation/'freeze.json']
    if len(by_case)!=len(rows):raise ValueError('Duplicate language cases')
    for availability,directory in gp['availability_runs'].items():
        suite_path=Path(plan['benchmark'])/f"{gp['split']}-opaque-{availability}.json";suite=load(suite_path)
        report_path=geometry/directory/'comparison.json';geometry_report=load(report_path)
        artifacts.extend([suite_path,report_path])
        if len(suite['cases'])!=plan['spans']:raise ValueError('Incomplete planned suite')
        for adapter in plan['unavailable_interfaces']:
            entries=[e for e in geometry_report['executions'] if e['adapter']==adapter]
            if len(entries)!=plan['spans']:raise ValueError('Incomplete unavailable-interface evidence')
            if any(e.get('result') and e['result']['status']=='ok' and e['result']['compatible'] is not None for e in entries):
                raise ValueError('Declared unavailable interface exposes compatibility')
        for case in suite['cases']:
            row=by_case[case['id']]
            if row['availability']!=availability:raise ValueError('Availability label differs')
            for adapter in plan['unavailable_interfaces']:
                old=row['tracks'][adapter]
                row['tracks'][adapter]=dict(status='not_available',reason='Geometric API never exposes compatibility',raw_context_track_status=old['status'])
            successful=[weight for weight in plan['weights'] if row['tracks'][f'local-profile-cost-weight-{weight}']['status']=='ok']
            if not successful:continue
            cache_path=evaluation/case['id']/'scores.json';cache=load(cache_path);artifacts.append(cache_path)
            scores={s['id']:s['token_log_score_sum'] for s in cache['scores']}
            truth=case['truth_id'] if availability=='present' else None
            for weight in successful:
                track=f'local-profile-cost-weight-{weight}';path=evaluation/case['id']/f'{track}.json'
                actual=check_report(load(path),scores,weight,truth);grade=row['tracks'][track]
                if any(actual[k]!=grade[k] for k in actual):raise ValueError('Recorded grade differs from actual CLI ranking')
                artifacts.append(path);checked.append(dict(case=case['id'],track=track,**actual))
    verify(freeze['files']);verify(load(a.plan.parent/'freeze.json'))
    with a.output.open('x') as f:
        json.dump(dict(rows=rows,**summarize(rows),all_frozen_hashes_verified=True,cli_reports_checked=len(checked),
            cli_checks=checked,source_files={str(p.resolve()):digest(p) for p in artifacts},
            interpretation='Actual CLI ranked order and tie groups independently checked from cached full-sequence language scores and saved physical costs. Scores/outcomes unchanged. Unavailable compatibility interfaces remain unknown on operational failures; original raw report preserved.'),f,indent=2);f.write('\n')
    print(json.dumps(dict(planned_cases=len(rows),cli_reports_checked=len(checked),unavailable_interfaces=plan['unavailable_interfaces'])))


if __name__=='__main__':main()
