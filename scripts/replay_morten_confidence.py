# Copyright (C) 2026 Toit contributors.
"""Replay native CSV-confidence selection with the same cached local T5 scores.

This is a declared integration through an upstream input field, not an invented
compatible set or native automatic LLM feature. Every zero-confidence replay must
exactly reproduce the saved upstream selected match before any adaptation runs.
"""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import sys
from types import SimpleNamespace

from audit_native_morten_decisions import decision


def load(path): return json.loads(Path(path).read_text())
def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def relative_weights(scores, ids):
    if set(scores)!=set(ids) or any(not math.isfinite(s) for s in scores.values()):
        raise ValueError('Common language scores must cover every supplied candidate')
    highest=max(scores.values());values={k:math.exp(v-highest) for k,v in scores.items()};total=math.fsum(values.values())
    return {k:v/total for k,v in values.items()}


def replay(loop,case,native,weights,scale):
    widths={m['text']:m['width'] for m in native['measurements']}
    if len(widths)!=len(native['measurements']):raise ValueError('Duplicate candidate surface in replay')
    class CachedFont:
        def getlength(self,text): return widths[text]
    candidates=[dict(name=m['text'],confidence=scale*weights.get(m['id'],0.),notes='') for m in native['measurements']]
    # Reproduce load_candidates' stable descending CSV-confidence order.
    candidates.sort(key=lambda c:c['confidence'],reverse=True)
    b=case['input']['targets'][0]['bounds']
    matches,_=loop(CachedFont(),1.,0.,candidates,[(b['x'],b['y'],native['target_width'],b['h'])],SimpleNamespace(shape=(1,1)))
    return json.loads(json.dumps(matches))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan)
    for path,sha in load(a.plan.parent/'freeze.json').items():
        if digest(path)!=sha:raise ValueError(f'Confidence replay protocol changed:{path}')
    root=Path(plan['geometry_root']);benchmark=Path(plan['benchmark']);language=Path(plan['language_evaluation'])
    language_summary=load(language/'summary.json');geometry_plan=load(root/'plan.json')
    if len(language_summary['rows'])!=2*plan['spans'] or not language_summary['all_frozen_hashes_verified']:
        raise ValueError('Complete common-language comparison required')
    if digest(Path(plan['repository'])/'unredactron.py')!=plan['native_source_sha256']:
        raise ValueError('Pinned native selection source changed')
    sys.path.insert(0,plan['adapter_directory'])
    from morten_unredactron import scoring_loop
    loop=scoring_loop(Path(plan['repository'])/'unredactron.py')
    adapter='morten-matched-catalog-adaptation';tool=geometry_plan['adapters'].index(adapter)
    labels={c['id']:r for r in load(benchmark/'controller/labels.json')['labels'] for c in r['cases']}
    rows=[];artifacts=[a.plan,Path(__file__),language/'summary.json']
    for availability,directory in geometry_plan['availability_runs'].items():
        suite_path=benchmark/f"{geometry_plan['split']}-opaque-{availability}.json";suite=load(suite_path)
        report_path=root/directory/'comparison.json';report=load(report_path)
        if report['suite_sha256']!=digest(suite_path) or len(report['executions'])!=plan['spans']*len(geometry_plan['adapters']):
            raise ValueError('Incomplete or unmatched geometry comparison')
        if any(p.get('error') for p in report['provenance']):raise ValueError('Failed native provenance')
        entries={e['case']:e for e in report['executions'] if e['adapter']==adapter}
        artifacts += [suite_path,report_path]
        for index,case in enumerate(suite['cases']):
            entry=entries[case['id']];base=dict(case=case['id'],document=labels[case['id']]['document'],availability=availability)
            if entry['error'] or (entry.get('result') or {}).get('status')!='ok':
                rows.append(dict(**base,status='failed_or_unsupported',variants={}));continue
            native_path=root/directory/f'case-{index:03}-repeat-00-tool-{tool:02}/work/native-result.json'
            native=load(native_path);artifacts.append(native_path)
            actual=decision(case,native,availability)
            zero=replay(loop,case,native,{},0.)
            if zero!=native['matches']:raise ValueError('Cached-width replay changed native zero-confidence output')
            cache=language/case['id']/'scores.json';weights={};available=cache.exists()
            if available:
                cached=load(cache);items=cached['request']['items'];candidates={c['id']:c['surfaces'][0]['text'] for c in case['input']['candidates']}
                if len(items)!=len(candidates) or {i['id']:i['text'] for i in items}!=candidates:
                    raise ValueError('Language cache candidate surfaces differ')
                if len(cached['scores'])!=len(candidates) or len({s['target'] for s in cached['scores']})!=1:
                    raise ValueError('One complete single-occurrence cache expected')
                weights=relative_weights({s['id']:s['token_log_score_sum'] for s in cached['scores']},candidates)
                artifacts.append(cache)
            variants={'native-zero':actual}
            for scale in plan['confidence_scales']:
                matches=replay(loop,case,native,weights,scale)
                selected=None
                if matches:
                    selected=next(m['id'] for m in native['measurements'] if m['text']==matches[0]['candidate'])
                    if not matches[0]['pct_error']<30:raise ValueError('Native raw-error output gate violated')
                variants[f't5-confidence-scale-{scale:g}']=dict(accepted=bool(matches),rejected=not matches,
                    selected_id=selected,selected_correct=availability=='present' and selected==case['truth_id'])
            rows.append(dict(**base,status='ok',lm_available=available,zero_replay_exact=True,variants=variants))
    if len(rows)!=2*plan['spans']:raise ValueError('Incomplete planned cases')
    methods=['native-zero',*[f't5-confidence-scale-{s:g}' for s in plan['confidence_scales']]]
    summary=[]
    for availability in ['present','absent']:
        subset=[r for r in rows if r['availability']==availability]
        for method in methods:
            values=[r['variants'][method] for r in subset if r['status']=='ok']
            summary.append(dict(availability=availability,method=method,cases=len(subset),statuses=dict(Counter(r['status'] for r in subset)),
                accepted=sum(v['accepted'] for v in values),rejected=sum(v['rejected'] for v in values),
                selected_correct=sum(v['selected_correct'] for v in values) if availability=='present' else None,
                wrong_accepted=sum(v['accepted'] for v in values) if availability=='absent' else None))
    with a.output.open('x') as f:
        json.dump(dict(rows=rows,summary=summary,files={str(p.resolve()):digest(p) for p in artifacts},plan=plan,
            interpretation='Actual pinned native selection loop, with cached native widths. Zero confidence must reproduce saved outputs. Relative softmax weights from identical T5 scores feed the existing CSV-confidence field at every declared scale; no compatible cutoff invented. Scale100 is a percentage-style stress control, not a documented native automatic AI default. Missing context falls back to native zero confidence. All variants reported, no test-based selection; failures not credited as rejection.'),f,indent=2);f.write('\n')
    for row in summary:print(row)


if __name__=='__main__':main()
