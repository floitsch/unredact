# Copyright (C) 2026 Toit contributors.
"""Replay existing pages and explicit links; this is not an accuracy benchmark."""
import argparse
import copy
import json
import math
from pathlib import Path
import subprocess

from evaluate_mosaic_cells import prepare_request


def load(path): return json.loads(Path(path).read_text())
def write(path,data): Path(path).write_text(json.dumps(data,indent=2)+'\n')


def run(command):
    result=subprocess.run(list(map(str,command)),capture_output=True,text=True,timeout=240)
    if result.returncode:raise RuntimeError(result.stderr)


def combine(binary,directory,reports):
    entries=[dict(id=f'observation-{i:02}',report=str(path.resolve())) for i,path in enumerate(reports)]
    entries.append(dict(id='duplicate-export',report=str(reports[0].resolve())))
    write(directory/'links.json',dict(group='explicit integration link assumption',occurrences=entries))
    run([binary,'combine-pixelation','--occurrences',directory/'links.json','--output',directory/'joint.json'])
    joint=load(directory/'joint.json')
    expected=[{s['id']:s['cost'] for s in load(path)['scores']} for path in reports]
    if joint['independent_observations']!=len(reports) or joint['ignored_occurrences']!=['duplicate-export']:
        raise ValueError('Duplicate counted as independent evidence')
    for score in joint['scores']:
        values=[r[score['id']] for r in expected]
        if any(v is None for v in values):
            if score['cost'] is not None:raise ValueError('Unsupported candidate became renderable')
        elif score['cost'] is None or abs(math.fsum(values)-score['cost'])>1e-8:
            raise ValueError('Independent-page joint cost differs from the expected sum')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=Path('target/release/unredact'))
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=False)
    previous=Path('runs/main-before-mosaic-integration-001/cli-replay')
    experimental=Path('runs/mosaic-cells-development-001/evaluation')
    solo=[]
    for source in sorted(previous.glob('c-*')):
        request=load(experimental/source.name/'request.json')
        directory=a.output/source.name;directory.mkdir()
        for mode in ['unit','learned']:
            target=directory/mode;target.mkdir();output=target/'ranking.json'
            command=[a.binary,'rank-pixelation','--project',source/'project','--profile',request['profile']['id'],
                     '--regions',source/'regions.json','--target-index',request['target_index'],
                     '--candidates',source/'candidates.json','--output',output]
            if mode=='learned':command+=['--learn-vertical']
            run(command)
            new,old=load(output),load(source/(mode+'.json'))
            if len(new['scores'])!=len(old['scores']):raise ValueError('Candidate count changed')
            for actual,expected in zip(new['scores'],old['scores']):
                if any(actual[k]!=v for k,v in expected.items()):raise ValueError('Original solo score fields changed')
            if any(new[k]!=old[k] for k in ['grid_analysis','cell_models']):raise ValueError('Solo physics changed')
            combine(a.binary,target,[output])
            solo.append(dict(case=source.name,mode=mode,candidates=len(new['scores'])))
    if len(solo)!=24:raise ValueError('Expected24 twelve-page integration replays')
    benchmark=Path('runs/multidoc-blinded-coordinates-001')
    labels=[r for r in load(benchmark/'controller/labels.json')['labels'] if r['split']=='development']
    groups={}
    for label in labels:groups.setdefault(label['text'],[]).append(label)
    groups={t:v for t,v in groups.items() if len({r['document'] for r in v})>1}
    indices={c['id']:i for i,c in enumerate(load(benchmark/'development-opaque-present.json')['cases'])}
    cases={c['id']:c for c in load(benchmark/'development-pixelate-8-present.json')['cases']}
    cross=[]
    for group_number,(_,labels) in enumerate(sorted(groups.items())):
        directory=a.output/f'cross-pdf-{group_number:02}';directory.mkdir();reports=[];bank=None
        for index,label in enumerate(labels):
            case_id=next(c['id'] for c in label['cases'] if c['condition']=='pixelate-8' and c['availability']=='present')
            reference_id=next(c['id'] for c in label['cases'] if c['condition']=='opaque' and c['availability']=='present')
            case=copy.deepcopy(cases[case_id])
            if bank is None:bank=case['input']['candidates']
            case['input']['candidates']=copy.deepcopy(bank)
            cached=Path('runs/multidoc-language-development-001/eval-8f2a')/f'case-{indices[reference_id]:03}-repeat-00-tool-00/work'
            request,project,_=prepare_request(benchmark,case,cached)
            target=directory/f'occurrence-{index:02}';target.mkdir();(target/'project').mkdir()
            write(target/'project/project.json',project);write(target/'regions.json',request['regions'])
            write(target/'candidates.json',dict(candidates=bank));output=target/'ranking.json'
            run([a.binary,'rank-pixelation','--project',target/'project','--profile',request['profile']['id'],
                 '--regions',target/'regions.json','--target-index',request['target_index'],'--candidates',target/'candidates.json',
                 '--learn-vertical','--output',output])
            reports.append(output)
        combine(a.binary,directory,reports)
        cross.append(dict(documents=sorted({r['document'] for r in labels}),occurrences=len(reports),candidates=len(bank)))
    write(a.output/'summary.json',dict(solo_replays=solo,cross_pdf_groups=cross,
        interpretation='Reused development integration fixtures and supplied identity links. Exact solo-score preservation, independent-page cost sums and duplicate exclusion; no accuracy estimate or automatic link-validation claim.'))
    print(json.dumps(dict(solo_replays=len(solo),cross_pdf_groups=len(cross))))


if __name__=='__main__':main()
