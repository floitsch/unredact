# Copyright (C) 2026 Toit contributors.
"""Audit a paired bounded-fit reevaluation without dropping old failures."""
import argparse
import json
from pathlib import Path
from summarize_linked_comparisons import metrics


def load(path):
    return json.loads(path.read_text())


def executions(root, expected_count):
    report = load(root/'comparison.json')
    rows = {(r['case'],r['adapter']):r for r in report['executions']}
    assert len(rows) == expected_count, 'Missing or repeated execution'
    work = {}
    files = list(root.glob('*/execution.json'))
    assert len(files) == expected_count
    for path in files:
        row = load(path)
        key = (row['case'],row['adapter'])
        assert rows[key] == row
        work[key] = path.parent/'work'
    return rows,work


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--old',type=Path,required=True)
    p.add_argument('--new',type=Path,required=True)
    a = p.parse_args()
    old,old_work = executions(a.old/'local-comparison',108)
    width,width_work = executions(a.new/'width-comparison',36)
    context,context_work = executions(a.new/'context-comparison',24)
    assert not width.keys() & context.keys()
    new = {**width,**context}
    work = {**width_work,**context_work}
    width_id = 'width-fringe-default-filtered'
    conditions,changes = {},[]
    for key,row in new.items():
        previous = old[key]
        assert row['input_sha256'] == previous['input_sha256'], 'Paired inputs changed'
        assert not row['error'] and row['result']['status']=='ok', (key,row['error'])
        change = dict(case=key[0],adapter=key[1],old_error=previous['error'],
                      old_metrics=previous['metrics'],new_metrics=row['metrics'])
        if not previous['error'] and previous['result']['status']=='ok':
            old_native = load(old_work[key]/'native-result.json')
            new_native = load(work[key]/'native-result.json')
            change['physical_report_identical'] = old_native == new_native
            if key in context:
                old_context = load(old_work[key]/'context-result.json')
                new_context = load(work[key]/'context-result.json')
                assert old_context['contexts'] == new_context['contexts'], 'Context changed'
        changes.append(change)
    for version,rows in [('old',{k:old[k] for k in new}),('bounded',new)]:
        for adapter in sorted({k[1] for k in rows}):
            for condition in ['first','second','joint']:
                selected=[r for (case,tool),r in rows.items() if tool==adapter and case.startswith(condition+'-')]
                if selected:
                    conditions[f'{version}::{condition}::{adapter}'] = metrics(selected)
    for key in context:
        assert load(work[key]/'native-result.json') == load(width_work[key[0],width_id]/'native-result.json')
    pairs=[]
    for i in range(6):
        for presence in ['present','absent']:
            names=[f'{part}-{presence}-pair-{i:03}' for part in ['first','second','joint']]
            reports=[load(width_work[name,width_id]/'native-result.json') for name in names]
            candidates=[{c['id']:c for c in r['candidates']} for r in reports]
            compatible=[{c['id'] for c in r['candidates'] if c['compatible']} for r in reports]
            assert compatible[2] == compatible[0] & compatible[1]
            assert [r['independent_observations'] for r in reports] == [1,1,2]
            delta=max(abs(c['evidence_score']-candidates[0][cid]['evidence_score']-candidates[1][cid]['evidence_score'])
                      for cid,c in candidates[2].items())
            assert delta < 1e-8
            pairs.append(dict(pair=f'pair-{i:03}',presence=presence,
                              compatible_counts=[len(s) for s in compatible],max_physical_cost_delta=delta))
    output = dict(conditions=conditions,changes=changes,pairs=pairs,
        audits=dict(paired_input_hashes_match=True,previously_successful_contexts_unchanged=True,
                    joint_physics_equal_across_language_modes=True,independent_width_costs_add_once=True),
        scope='Paired development reevaluation on six reused linked identities, not a new holdout. Baseline failures retained. Language modes reevaluated only jointly; no new single-view language accuracy claim.')
    (a.new/'summary.json').write_text(json.dumps(output,indent=2)+'\n')
    for key,value in conditions.items():
        print(key, {k:v for k,v in value.items() if k!='failures'}, 'failures',len(value['failures']))


if __name__=='__main__':
    main()
