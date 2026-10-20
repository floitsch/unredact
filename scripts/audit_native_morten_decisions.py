# Copyright (C) 2026 Toit contributors.
"""Audit actual Unredactron selected-match decisions, not invented compatibility sets."""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path


def load(path): return json.loads(Path(path).read_text())
def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def decision(case, native, availability):
    candidates={c['id']:c['surfaces'][0]['text'] for c in case['input']['candidates']}
    if availability not in {'present','absent'} or (case['truth_id'] in candidates)!=(availability=='present'):
        raise ValueError('Availability label differs from supplied candidate vocabulary')
    measured=native['measurements']
    if len(measured)!=len(candidates) or {m['id'] for m in measured}!=candidates.keys():
        raise ValueError('Native candidate coverage differs')
    if any(not math.isfinite(m['score']) or m['score']<0 for m in measured):
        raise ValueError('Invalid native width error')
    if any(m['text']!=candidates[m['id']] for m in measured):
        raise ValueError('Native candidate surface differs')
    matches=native['matches'];minimum=min(m['score'] for m in measured)
    if len(matches)>1 or bool(matches)!=(minimum<30):
        raise ValueError('Native strict30-percent acceptance gate changed')
    selected=None
    if matches:
        ids=[m['id'] for m in measured if m['text']==matches[0]['candidate']]
        if len(ids)!=1:raise ValueError('Native selected text does not uniquely identify a supplied candidate')
        selected=ids[0]
        item=next(m for m in measured if m['id']==selected)
        if abs(item['score']-minimum)>1e-9 or abs(matches[0]['pct_error']-item['score'])>1e-9:
            raise ValueError('Native match is not its minimum-error neutral-confidence candidate')
    return dict(status='ok',accepted=bool(matches),selected_id=selected,
        selected_correct=availability=='present' and selected==case['truth_id'],
        rejected=not matches,minimum_width_error_percent=minimum)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan);root=Path(plan['geometry_root']);benchmark=Path(plan['benchmark'])
    for path,sha in load(a.plan.parent/'freeze.json').items():
        if digest(path)!=sha:raise ValueError('Registered native-output audit changed')
    geometry_plan=load(root/'plan.json');adapter='morten-matched-catalog-adaptation'
    tool=geometry_plan['adapters'].index(adapter);labels={c['id']:r for r in load(benchmark/'controller/labels.json')['labels'] for c in r['cases']}
    rows=[];artifacts=[a.plan,Path(__file__),benchmark/'controller/labels.json']
    for availability,directory in geometry_plan['availability_runs'].items():
        suite_path=benchmark/f"{geometry_plan['split']}-opaque-{availability}.json"
        report_path=root/directory/'comparison.json';report=load(report_path);suite=load(suite_path)
        expected=len(suite['cases'])*len(geometry_plan['adapters'])
        if len(suite['cases'])!=plan['spans'] or len(report['executions'])!=expected or report['suite_sha256']!=digest(suite_path):
            raise ValueError('Complete planned geometry suite required')
        if any(p.get('error') for p in report['provenance']):raise ValueError('Invalid runtime provenance')
        executions={e['case']:e for e in report['executions'] if e['adapter']==adapter}
        if set(executions)!={c['id'] for c in suite['cases']}:raise ValueError('Missing/duplicate native executions')
        artifacts += [suite_path,report_path]
        for index,case in enumerate(suite['cases']):
            execution=executions[case['id']];result=execution.get('result') or {}
            if execution['error'] or result.get('status')!='ok':
                value=dict(status='failed_or_unsupported',reason=execution['error'] or result.get('reason'))
            else:
                native_path=root/directory/f'case-{index:03}-repeat-00-tool-{tool:02}/work/native-result.json'
                value=decision(case,load(native_path),availability);artifacts.append(native_path)
            rows.append(dict(case=case['id'],document=labels[case['id']]['document'],availability=availability,**value))
    if len(rows)!=2*plan['spans']:raise ValueError('All planned native decisions required')
    summaries=[]
    for availability in ['present','absent']:
        relevant=[r for r in rows if r['availability']==availability]
        summaries.append(dict(availability=availability,cases=len(relevant),statuses=dict(Counter(r['status'] for r in relevant)),
            accepted=sum(r.get('accepted',False) for r in relevant),rejected=sum(r.get('rejected',False) for r in relevant),
            selected_correct=sum(r.get('selected_correct',False) for r in relevant) if availability=='present' else None,
            wrong_accepted=sum(r.get('accepted',False) for r in relevant) if availability=='absent' else None))
    with a.output.open('x') as f:
        json.dump(dict(rows=rows,summary=summaries,source_files={str(p.resolve()):digest(p) for p in artifacts},
            interpretation='Actual selected-match output from pinned upstream code under neutral CSV confidence. Its strict30-percent gate is not a vocabulary-wide compatible set and is not a calibrated identity probability. Native tie selection follows supplied candidate order. Failures are not credited as rejection. Ranking and common-language controls remain unchanged.'),f,indent=2);f.write('\n')
    for row in summaries:print(row)


if __name__=='__main__':main()
