# Copyright (C) 2026 Toit contributors.
"""Audit a completed language run; preserve unavailable compatibility APIs as unknown.

A context failure affects all operational language tracks. It does not turn an
upstream interface that never exposes a compatible set into a measured zero.
The raw evaluator report and all candidate scores remain unchanged.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
from evaluate_multidoc_language_v2 import summarize, verify


def normalize(rows, unavailable):
    result=copy.deepcopy(rows)
    for row in result:
        for track in unavailable:
            previous=row['tracks'][track]
            row['tracks'][track]=dict(status='not_available',reason='Geometric API never exposes compatibility',
                                      raw_context_track_status=previous['status'])
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evaluation',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    freeze=json.loads((a.evaluation/'freeze.json').read_text())
    verify(freeze['files'])
    raw_path=a.evaluation/'summary.json';raw=json.loads(raw_path.read_text())
    if len(raw['rows'])!=192 or len({r['case'] for r in raw['rows']})!=192 or not raw['all_frozen_hashes_verified']:
        raise ValueError('Full192-case completion required')
    geometry=Path(freeze['plan']['geometry_root']);plan=json.loads((geometry/'plan.json').read_text())
    executions=[]
    for directory in plan['availability_runs'].values():
        executions+=json.loads((geometry/directory/'comparison.json').read_text())['executions']
    adapters={e['adapter'] for e in executions}
    unavailable={adapter for adapter in adapters if all(e['result']['compatible'] is None for e in executions if e['adapter']==adapter)}
    rows=normalize(raw['rows'],unavailable)
    report=dict(schema_version=1,raw_summary_sha256=hashlib.sha256(raw_path.read_bytes()).hexdigest(),
                script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),unavailable_interfaces=sorted(unavailable),
                rows=rows,**summarize(rows),all_frozen_hashes_verified=True,
                interpretation=freeze['plan']['interpretation'],
                correction='Unavailable geometry APIs remain unknown even when a case also lacks visible context. No candidate/model scores, ranks, or supported-track outcomes changed.')
    with a.output.open('x') as f:json.dump(report,f,indent=2);f.write('\n')
    for row in report['metrics']:
        if row['breakdown']=='all':print(row)


if __name__=='__main__':main()
