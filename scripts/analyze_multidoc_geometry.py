# Copyright (C) 2026 Toit contributors.
"""Analyze completed matched geometry runs, retaining ties and unsupported cases.

This report never treats an unexposed compatibility decision as an empty set.
It distinguishes unconditional task success from a both-tools-supported control.
"""
import argparse
from collections import Counter,defaultdict
import hashlib
import json
from pathlib import Path
import statistics

from summarize_multidoc_benchmark import cluster_interval


def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def aggregate(rows):
    ok=[r for r in rows if r['status']=='ok'];present=rows[0]['availability']=='present'
    exposed=[r for r in ok if r['metrics']['compatible_candidates'] is not None]
    report=dict(cases=len(rows),statuses=dict(Counter(r['status'] for r in rows)),
                unique_correct_first=sum(r['metrics'].get('unique_correct_first',False) for r in rows),
                truth_in_best_group=sum(r['metrics'].get('truth_in_best_group',False) for r in rows),
                compatibility_exposed_successes=len(exposed),compatibility_unreported_successes=len(ok)-len(exposed),
                median_compatible_candidates=statistics.median(r['metrics']['compatible_candidates'] for r in exposed) if exposed else None,
                mean_elapsed_seconds=statistics.mean(r['elapsed_seconds'] for r in rows))
    # A missing API field is unknown, including for an otherwise successful tool.
    can_report=bool(exposed) and len(exposed)==len(ok)
    if present:
        report['truth_compatible']=sum(r['metrics']['truth_compatible'] is True for r in exposed) if can_report else None
        report['truth_top5_guaranteed']=sum(r['metrics'].get('truth_rank') is not None and r['metrics']['truth_rank'][1]<=5 for r in rows)
        report['truth_top5_possible']=sum(r['metrics'].get('truth_rank') is not None and r['metrics']['truth_rank'][0]<=5 for r in rows)
    else:
        report['empty_compatible_set']=sum(r['metrics']['no_compatible'] is True for r in exposed) if can_report else None
        report['single_wrong_compatible']=sum(r['metrics']['single_wrong_compatible'] is True for r in exposed) if can_report else None
    grouped=defaultdict(list)
    for r in rows:grouped[r['source_group']].append(float(r['metrics'].get('unique_correct_first',False)))
    if present:
        means={g:statistics.mean(grouped[g]) for g in sorted(grouped)}
        report.update(document_macro_first=statistics.mean(means.values()),document_95_interval=cluster_interval(list(means.values())),per_document_first=means)
    return report


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--benchmark',type=Path,required=True);p.add_argument('--comparison',type=Path,action='append',required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();root=a.benchmark
    manifest=json.loads((root/'manifest.json').read_text())
    if manifest['label_review']!='verified':raise ValueError('Unverified labels')
    labels=json.loads((root/'controller/labels.json').read_text())['labels']
    metadata={c['id']:(r,c) for r in labels for c in r['cases']}
    suites={}
    for name,info in manifest['suites'].items():
        path=root/(name+'.json')
        if digest(path)!=info['sha256']:raise ValueError('Suite changed')
        suites[info['sha256']]=(name,path)
    rows=[];seen=set();sources=[];toolsets=[]
    for path in a.comparison:
        report=json.loads(path.read_text());name,suite_path=suites[report['suite_sha256']]
        suite=json.loads(suite_path.read_text());cases={c['id']:c for c in suite['cases']}
        tools=set(report['summary']);toolsets.append(tools)
        expected={(t,c,0) for t in tools for c in cases}
        actual={(e['adapter'],e['case'],e['repetition']) for e in report['executions']}
        if expected!=actual or len(actual)!=len(report['executions']):raise ValueError('Incomplete/duplicated execution coverage or unexpected repetitions')
        if any(p.get('error') for p in report['provenance']):raise ValueError('Failed runtime/source provenance')
        hashes={}
        for e in report['executions']:
            identity=(e['adapter'],e['case'],e['repetition'])
            if identity in seen:raise ValueError('Duplicate execution across reports')
            seen.add(identity)
            if hashes.setdefault(e['case'],e['input_sha256'])!=e['input_sha256']:raise ValueError('Unmatched adapter inputs')
            label,meta=metadata[e['case']]
            status=(e.get('result') or {}).get('status','error')
            if e.get('error'):status='error'
            metrics=e.get('metrics') or {}
            if status=='ok' and not metrics:raise ValueError('Successful execution lacks metrics')
            rows.append(dict(adapter=e['adapter'],case=e['case'],status=status,metrics=metrics,
                elapsed_seconds=e['elapsed_seconds'],error=e.get('error') or (e.get('result') or {}).get('reason'),
                source_group=label['source_group'],document=label['document'],page=label['page'],kind=label['kind'],
                split=label['split'],word_count=label['count'],text=label['text'],condition=meta['condition'],availability=meta['availability']))
        sources.append(dict(path=str(path.resolve()),sha256=digest(path),suite=name,suite_sha256=report['suite_sha256']))
    if any(s!=toolsets[0] for s in toolsets):raise ValueError('Different adapter sets across input reports')
    buckets=defaultdict(list)
    for row in rows:
        for breakdown in [('all','all'),('source_kind',row['kind']),('word_count',str(row['word_count'])),('document',row['document'])]:
            buckets[(row['split'],row['condition'],row['availability'],*breakdown,row['adapter'])].append(row)
    summaries=[dict(split=k[0],condition=k[1],availability=k[2],breakdown=k[3],value=k[4],adapter=k[5],**aggregate(v)) for k,v in sorted(buckets.items())]
    paired=[]
    for split,condition in sorted({(r['split'],r['condition']) for r in rows if r['availability']=='present'}):
        relevant={ (r['adapter'],r['case']):r for r in rows if r['split']==split and r['condition']==condition and r['availability']=='present'}
        tools=sorted(toolsets[0]);case_ids={c for _,c in relevant}
        for i,first in enumerate(tools):
            for second in tools[i+1:]:
                diffs=defaultdict(list);both=[]
                for case in sorted(case_ids):
                    x,y=relevant[first,case],relevant[second,case]
                    difference=int(x['metrics'].get('unique_correct_first',False))-int(y['metrics'].get('unique_correct_first',False))
                    diffs[x['source_group']].append(difference)
                    if x['status']==y['status']=='ok':both.append(difference)
                means=[statistics.mean(diffs[g]) for g in sorted(diffs)]
                paired.append(dict(split=split,condition=condition,first=first,second=second,cases=len(case_ids),documents=len(means),
                                   document_macro_difference=statistics.mean(means),document_95_interval=cluster_interval(means),
                                   both_successful_cases=len(both),both_successful_micro_difference=statistics.mean(both) if both else None))
    output=dict(schema_version=1,sources=sources,metrics=summaries,paired=paired,rows=rows,
        interpretation='Whole completed suites only. Failures/unsupported count as incorrect for unconditional first rank. Missing compatibility output remains null; no arbitrary cutoff added. Paired intervals resample documents, not spans. Both-successful control is conditional and secondary. Development results cannot establish held-out superiority; dictionary distractors are weak evidence for language-assisted performance.')
    with a.output.open('x') as f:json.dump(output,f,indent=2);f.write('\n')
    for row in summaries:
        if row['breakdown']=='all':print(row['availability'],row['adapter'],'cases',row['cases'],'first',row['unique_correct_first'],'status',row['statuses'],'truth compatible',row.get('truth_compatible'))


if __name__=='__main__':main()
