# Copyright (C) 2026 Toit contributors.
"""Evaluate the fixed pixelation scorer on all independent test-document spans.

Uses frozen opaque-reference typography only where every outside-mask pixel is
identical. Both unit and visible-training height scales are evaluated. The old
single-crop scorer is a separate control. No test result selects parameters.
"""
import argparse
import copy
import json
import os
from pathlib import Path
import subprocess
import time

from evaluate_mosaic_cells import load, write, digest, prepare_request, summarize
from evaluate_common_language import truth_rank


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan)
    root=Path(plan['reference_geometry_root']);benchmark=Path(plan['benchmark'])
    geo=load(root/'eval-8de1/comparison.json');suite_path=benchmark/'test-opaque-present.json'
    if len(geo['executions'])!=576 or geo['suite_sha256']!=digest(suite_path) or any(p.get('error') for p in geo['provenance']):
        raise ValueError('Complete pinned test reference geometry required')
    reference={c['id']:i for i,c in enumerate(load(suite_path)['cases'])}
    labels={c['id']:r for r in load(benchmark/'controller/labels.json')['labels'] for c in r['cases'] if r['split']=='test'}
    out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    files=load(Path(plan['experiment_root'])/'freeze.json')
    artifacts=[root/'eval-8de1/comparison.json',suite_path]
    jobs=[]
    for condition in ['pixelate-8','pixelate-16']:
        for availability in ['present','absent']:
            path=benchmark/f'test-{condition}-{availability}.json';artifacts.append(path)
            suite=load(path)
            if len(suite['cases'])!=192:raise ValueError('Full192-case stratum required')
            for case in suite['cases']:
                label=labels[case['id']]
                ref_id=next(c['id'] for c in label['cases'] if c['condition']=='opaque' and c['availability']=='present')
                execution=next(e for e in geo['executions'] if e['case']==ref_id and e['adapter']=='local-profile-cost')
                if execution['error'] or execution['result']['status']!='ok':
                    jobs.append(dict(case=case,label=label,condition=condition,availability=availability,
                                     failure=execution['error'] or execution['result'].get('reason','Reference typography failed')))
                    continue
                work=root/'eval-8de1'/f'case-{reference[ref_id]:03}-repeat-00-tool-00/work'
                request,project,old_image=prepare_request(benchmark,case,work)
                directory=out/case['id'];directory.mkdir();(directory/'project').mkdir()
                write(directory/'project/project.json',project)
                write(directory/'regions.json',request['regions'])
                write(directory/'candidates.json',dict(candidates=case['input']['candidates']))
                artifacts += [work/'project/project.json',old_image,Path(request['image']),directory/'project/project.json',
                              directory/'regions.json',directory/'candidates.json']
                artifacts += [Path(f['font']) for f in request['profile']['fits']]
                jobs.append(dict(case=case,label=label,condition=condition,availability=availability,directory=directory,
                                 profile=request['profile']['id'],target_index=request['target_index']))
    if len(jobs)!=768:raise ValueError('Full768-case evaluation required')
    files.update({str(path.resolve()):digest(path) for path in artifacts})
    for path,sha in files.items():
        if digest(path)!=sha:raise ValueError(f'Frozen input changed:{path}')
    write(out/'freeze.json',dict(files=files,plan=plan))
    rows=[]
    for job in jobs:
        case,label=job['case'],job['label']
        row=dict(case=case['id'],document=label['document'],kind=label['kind'],condition=job['condition'],
                 availability=job['availability'],engines={})
        if 'failure' in job:
            row['engines']={e:dict(status='error',reason=job['failure']) for e in ['legacy','unit','shared_cells']}
        else:
            d=job['directory']
            current=[plan['binary'],'rank-pixelation','--project',d/'project','--profile',job['profile'],
                     '--regions',d/'regions.json','--target-index',job['target_index'],'--candidates',d/'candidates.json',
                     '--min-block','4','--max-block','20']
            old=[plan['legacy_binary'],'solve','--project',d/'project','--group','comparison',
                 '--candidates',d/'candidates.json','--limit','300']
            for engine,command in [('legacy',old),('unit',current),('shared_cells',current+['--learn-vertical'])]:
                start=time.monotonic()
                try:
                    proc=subprocess.run(list(map(str,command)),capture_output=True,text=True,timeout=240,
                                        env={**os.environ,'OMP_THREAD_LIMIT':'1'})
                    (d/f'{engine}.stderr').write_text(proc.stderr);(d/f'{engine}.json').write_text(proc.stdout)
                    if proc.returncode:result=dict(status='error',reason=proc.stderr[-2000:])
                    else:
                        report=json.loads(proc.stdout)
                        scores=({c['id']:-c['evidence_score'] for c in report['candidates']} if engine=='legacy' else
                                {c['id']:-c['cost'] for c in report['scores'] if c['cost'] is not None})
                        if engine!='legacy' and {c['id'] for c in report['scores']}!={c['id'] for c in case['input']['candidates']}:
                            raise ValueError('Candidate coverage changed')
                        result=dict(status='ok',renderable_candidates=len(scores),truth_rank=
                                    truth_rank(scores,case['truth_id']) if job['availability']=='present' else None)
                except subprocess.TimeoutExpired:result=dict(status='timeout',reason='Fixed240-second timeout')
                result['elapsed_seconds']=time.monotonic()-start;row['engines'][engine]=result
            write(d/'evaluation.json',row)
        rows.append(row);write(out/'progress.json',dict(completed=len(rows),planned=768))
        print(json.dumps(dict(completed=len(rows),case=case['id'])),flush=True)
    for path,sha in files.items():
        if digest(path)!=sha:raise ValueError(f'Frozen input changed:{path}')
    unit_control=copy.deepcopy(rows)
    for row in unit_control:row['engines']['legacy']=row['engines']['unit']
    write(out/'summary.json',dict(rows=rows,**summarize(rows),unit_control=summarize(unit_control),
        all_frozen_hashes_verified=True,interpretation=plan['interpretation']))


if __name__=='__main__':main()
