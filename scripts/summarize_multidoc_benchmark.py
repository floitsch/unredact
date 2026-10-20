# Copyright (C) 2026 Toit contributors.
"""Report document-macro accuracy and paired document-cluster bootstrap intervals.

Missing executions are never silently removed. Only complete strata receive
intervals. Failures/unsupported executions count as incorrect, separately tallied.
"""
import argparse
from collections import defaultdict
import json
import hashlib
from pathlib import Path

import numpy as np


def cluster_interval(values, seed=41762, draws=10000):
    """One equally weighted value per source group; no span-level resampling."""
    data = np.asarray(values,dtype=float)
    if len(data) < 2:
        return None
    rng = np.random.default_rng(seed)
    means = data[rng.integers(0,len(data),size=(draws,len(data)))].mean(axis=1)
    return list(map(float,np.quantile(means,[.025,.975])))


def summarize(case_index, executions, adapter_ids):
    rows = {}
    for execution in executions:
        k = (execution['adapter'],execution['case'],execution['repetition'])
        if k in rows:
            raise ValueError('Duplicate execution; retries are not additional trials')
        if execution['case'] not in case_index:
            raise ValueError('Execution is not in this benchmark')
        rows[k] = execution
    strata = defaultdict(list)
    for identity,meta in case_index.items():
        strata[(meta['split'],meta['kind'],meta['condition'],meta['availability'])].append(identity)
    report = []
    for stratum, identities in sorted(strata.items()):
        configurations = {}; group_values = {}
        for adapter in adapter_ids:
            grouped = defaultdict(list); failed=missing=unsupported=0; successes=0
            for identity in identities:
                trials=[v for (a,c,_),v in rows.items() if a==adapter and c==identity]
                if not trials:
                    missing += 1; continue
                values=[]
                for trial in trials:
                    status=(trial.get('result') or {}).get('status')
                    metric=trial.get('metrics') or {}
                    if status != 'ok':
                        failed += 1; unsupported += status=='unsupported'; value=0.
                    elif stratum[3]=='present':
                        value=float(metric['unique_correct_first'])
                    else:
                        # No compatibility decision is not evidence of abstention.
                        value=float(metric.get('no_compatible') is True)
                    values.append(value)
                successes += sum(values)/len(values)
                grouped[case_index[identity]['source_group']].append(sum(values)/len(values))
            per_group={g:float(np.mean(v)) for g,v in grouped.items()}
            group_values[adapter]=per_group
            complete=missing==0
            configurations[adapter]=dict(planned_cases=len(identities),completed_cases=len(identities)-missing,
                missing_cases=missing,failed_executions=failed,unsupported_executions=unsupported,
                complete=complete,per_source_group=per_group,
                case_micro_accuracy=successes/len(identities) if complete else None,
                document_macro_accuracy=float(np.mean(list(per_group.values()))) if complete and per_group else None,
                cluster_95_interval=cluster_interval(list(per_group.values())) if complete else None)
        paired=[]
        for i,a in enumerate(adapter_ids):
            for b in adapter_ids[i+1:]:
                if not(configurations[a]['complete'] and configurations[b]['complete']):
                    continue
                differences=[group_values[a][g]-group_values[b][g] for g in sorted(group_values[a])]
                paired.append(dict(first=a,second=b,macro_difference=float(np.mean(differences)),
                                   cluster_95_interval=cluster_interval(differences),groups=len(differences)))
        report.append(dict(split=stratum[0],kind=stratum[1],condition=stratum[2],availability=stratum[3],
                           metric='unique_correct_first' if stratum[3]=='present' else 'empty_compatible_set_proxy',
                           configurations=configurations,paired=paired))
    return report


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--benchmark',type=Path,required=True)
    p.add_argument('--comparison',type=Path,action='append',required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--allow-provisional-labels',action='store_true')
    args=p.parse_args()
    labels=json.loads((args.benchmark/'controller/labels.json').read_text())['labels']
    index={c['id']:{**r,**c} for r in labels for c in r['cases']}
    manifest=json.loads((args.benchmark/'manifest.json').read_text())
    if manifest['label_review'] != 'verified' and not args.allow_provisional_labels:
        raise ValueError('Labels have not been verified; use --allow-provisional-labels only for pipeline debugging')
    suite_hashes=set()
    for name,info in manifest['suites'].items():
        actual=hashlib.sha256((args.benchmark/(name+'.json')).read_bytes()).hexdigest()
        if actual!=info['sha256']:raise ValueError('Benchmark suite changed')
        suite_hashes.add(actual)
    executions=[]; adapters=set()
    for path in args.comparison:
        data=json.loads(path.read_text())
        if data['suite_sha256'] not in suite_hashes:raise ValueError('Comparison used a different suite')
        executions.extend(data['executions'])
        adapters.update(e['adapter'] for e in data['executions'])
    result=dict(schema_version=1,strata=summarize(index,executions,sorted(adapters)),
        label_review=json.loads((args.benchmark/'manifest.json').read_text())['label_review'],
        interpretation='Development and test documents remain separate. Conditions and availability controls are paired, not independent trials. Source groups receive equal weight. Bootstrap intervals describe this source sample only; few documents and pretrained-model exposure limit generalization. Absent-case no-compatible is reported only as an operational abstention proxy, not certainty about absence.')
    with args.output.open('x') as f:json.dump(result,f,indent=2);f.write('\n')


if __name__=='__main__':main()
