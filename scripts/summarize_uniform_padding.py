# Copyright (C) 2026 Toit contributors.
"""Report prior ablations with explicit credit for unresolved best-group ties."""
import argparse
import json
import math
from pathlib import Path
import statistics

from summarize_multidoc_benchmark import cluster_interval


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--evaluation',type=Path,required=True);p.add_argument('--benchmark',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    summary=json.loads((a.evaluation/'summary.json').read_text());assert summary['all_frozen_files_verified']
    rows=summary['rows'];assert len(rows)==192
    labels=json.loads((a.benchmark/'controller/labels.json').read_text())['labels']
    truths={c['id']:r['text'] for r in labels for c in r['cases']}
    results=[]
    for bank in ['original_dictionary','natural_visible_development']:
        modes={}
        for mode in ['existing_profile_cost','uniform_padding_prior']:
            records=[]
            for i,row in enumerate(r for r in rows if r['availability']=='present'):
                metric=row['tracks'][bank][mode];assert metric['execution_status']=='ok'
                physical=json.loads((a.evaluation/f'present-{i:03}'/f'{bank}-{mode}'/'physical.json').read_text())
                rank=metric['truth_rank'];credit=1/rank[1] if rank and rank[0]==1 else 0.
                records.append(dict(case=row['case'],document=row['document'],unique_first=metric['unique_correct_first'],fractional_best_group_credit=credit,
                                    stored_arbitrary_first=physical['candidates'][0]['name']==truths[row['case']]))
            modes[mode]=dict(unique_first=sum(r['unique_first'] for r in records),fractional_best_group_credit=math.fsum(r['fractional_best_group_credit'] for r in records),
                             stored_arbitrary_first=sum(r['stored_arbitrary_first'] for r in records),rows=records)
        documents={}
        for doc in sorted({r['document'] for r in rows}):
            before=[r for r in modes['existing_profile_cost']['rows'] if r['document']==doc]
            after=[r for r in modes['uniform_padding_prior']['rows'] if r['document']==doc]
            assert [r['case'] for r in before]==[r['case'] for r in after]
            documents[doc]=dict(unique_first_difference=statistics.mean(int(x['unique_first'])-int(y['unique_first']) for x,y in zip(after,before)),
                                fractional_credit_difference=statistics.mean(x['fractional_best_group_credit']-y['fractional_best_group_credit'] for x,y in zip(after,before)))
        results.append(dict(bank=bank,modes=modes,documents=documents,
                            unique_first_95_interval=cluster_interval([d['unique_first_difference'] for d in documents.values()]),
                            fractional_credit_95_interval=cluster_interval([d['fractional_credit_difference'] for d in documents.values()])))
    with a.output.open('x') as f:json.dump(dict(results=results,interpretation='Fractional credit is expected correct-first count under uniform random selection within each best group, not observed recovery. Unique first counts alone reward breaking honest ties. Natural-bank fractional credit decreases; no consistent accuracy gain established. All results are development-only.'),f,indent=2);f.write('\n')


if __name__=='__main__':main()
