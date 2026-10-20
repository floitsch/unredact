# Copyright (C) 2026 Toit contributors.
"""Learn rendering height from visible training words; evaluate all development mosaics.

Unit-scale replay must reproduce every prior candidate score exactly. Learned
scales use no target dimensions or hidden labels. Validation/audit words report
height errors without selecting the scale or excluding difficult targets.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import time

from evaluate_common_language import truth_rank
from evaluate_mosaic_cells import summarize


def load(path): return json.loads(Path(path).read_text())
def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def write(path,data): Path(path).write_text(json.dumps(data,indent=2,allow_nan=False)+'\n')


def execute(command, output, timeout=240):
    start=time.monotonic()
    try:
        result=subprocess.run(list(map(str,command)),capture_output=True,text=True,timeout=timeout)
    except subprocess.TimeoutExpired:
        return None,dict(status='timeout',reason='Fixed240-second execution limit',elapsed_seconds=time.monotonic()-start)
    output.with_suffix('.stderr').write_text(result.stderr)
    output.write_text(result.stdout)
    if result.returncode:
        return None,dict(status='error',reason=result.stderr[-2000:],elapsed_seconds=time.monotonic()-start)
    return json.loads(result.stdout),dict(status='ok',elapsed_seconds=time.monotonic()-start)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan)
    previous=Path(plan['previous_evaluation']);geometry=Path(plan['geometry_root']);benchmark=Path(plan['benchmark'])
    source=Path(plan['source']);ranker=Path(plan['ranker']);learner=Path(plan['learner'])
    reference=load(previous/'summary.json')
    if len(reference['rows'])!=384 or not reference['all_frozen_hashes_verified']:raise ValueError('Complete prior cohort required')
    files=load(previous/'freeze.json')['files']
    for path,sha in files.items():
        if digest(path)!=sha:raise ValueError('Prior dependency changed')
    labels={c['id']:r for r in load(benchmark/'controller/labels.json')['labels'] for c in r['cases']}
    suite=load(benchmark/'development-opaque-present.json')
    reference_indices={c['id']:i for i,c in enumerate(suite['cases'])}
    out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    (out/'profiles').mkdir()
    artifacts=[a.plan,Path(__file__),Path(__file__).with_name('evaluate_mosaic_cells.py'),
        Path(__file__).with_name('evaluate_common_language.py'),ranker,learner,previous/'summary.json',
        *source.glob('src/*.rs'),*source.glob('native/*'),*source.glob('examples/*.rs'),
        source/'Cargo.toml',source/'Cargo.lock',source/'build.rs']
    jobs=[];profiles={}
    for row in reference['rows']:
        label=labels[row['case']]
        old_id=next(c['id'] for c in label['cases'] if c['condition']=='opaque' and c['availability']=='present')
        cached=geometry/'eval-8f2a'/f'case-{reference_indices[old_id]:03}-repeat-00-tool-00/work/project'
        project=load(cached/'project.json');target=project['targets'][0]
        page=next(p for p in project['pages'] if p['id']==target['page'])
        profile=next(p for p in project['profiles'] if p['id']==target['profile'])
        identity=hashlib.sha256(json.dumps(dict(profile=profile,words=page['words']),sort_keys=True).encode()).hexdigest()
        request_path=previous/row['case']/'request.json';request=load(request_path)
        if request['profile']['fits']!=profile['fits']:raise ValueError('Typography differs from reference')
        if identity not in profiles:
            result,status=execute([learner,cached],out/'profiles'/f'{identity}.json')
            if status['status']!='ok':raise RuntimeError(status)
            fits=next(p['fits'] for p in result if p['profile']==profile['id'])
            profiles[identity]=dict(fits=fits,document=label['document'],page=label['page'],project=str(cached),
                                    source_sha256=digest(cached/'project.json'))
        directory=out/row['case'];directory.mkdir()
        for mode,factors in [('unit',[1.]*len(profile['fits'])),('learned',[f['scale'] for f in profiles[identity]['fits']])]:
            amended=copy.deepcopy(request);amended['vertical_scales']=factors
            write(directory/f'{mode}-request.json',amended)
            artifacts.append(directory/f'{mode}-request.json')
        artifacts += [cached/'project.json',request_path,previous/row['case']/'shared_cells.json',out/'profiles'/f'{identity}.json']
        jobs.append(dict(row=row,label=label,directory=directory,profile=identity))
    if len(profiles)!=12:raise ValueError('Expected12 unique page typography profiles, not repeated target-level audits')
    write(out/'visible-height-audits.json',profiles)
    artifacts.append(out/'visible-height-audits.json')
    files.update({str(path.resolve()):digest(path) for path in artifacts})
    write(out/'freeze.json',dict(files=files,plan=plan))
    rows=[]
    for job in jobs:
        old,label,directory=job['row'],job['label'],job['directory']
        row={k:old[k] for k in ['case','document','kind','condition','availability']};row['engines']={}
        for mode,engine in [('unit','legacy'),('learned','shared_cells')]:
            result,status=execute([ranker,directory/f'{mode}-request.json'],directory/f'{mode}.json')
            if status['status']=='ok':
                if mode=='unit':
                    original=load(previous/old['case']/'shared_cells.json')
                    for key in ['scores','grid_analysis','cell_models']:
                        if result[key]!=original[key]:raise ValueError('Unit-scale replay changed prior candidate scores or physics')
                scores={s['id']:-s['cost'] for s in result['scores'] if s['cost'] is not None}
                truth=next(c['id'] for c in load(directory/f'{mode}-request.json')['candidates']
                           if any(s['text']==label['text'] for s in c['surfaces'])) if old['availability']=='present' else None
                status.update(renderable_candidates=len(scores),truth_rank=truth_rank(scores,truth) if truth else None)
            row['engines'][engine]=status
        rows.append(row);write(directory/'evaluation.json',row)
        write(out/'progress.json',dict(completed=len(rows),planned=384))
        print(json.dumps(dict(completed=len(rows),case=row['case'])),flush=True)
    for path,sha in files.items():
        if digest(path)!=sha:raise ValueError(f'Frozen dependency changed:{path}')
    write(out/'summary.json',dict(rows=rows,**summarize(rows),profiles=profiles,all_frozen_hashes_verified=True,
        engine_labels=dict(legacy='same shared-cell scorer, unit vertical scale',shared_cells='same shared-cell scorer, visible-training vertical scale'),
        interpretation=plan['interpretation']))


if __name__=='__main__':main()
