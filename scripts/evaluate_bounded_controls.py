# Copyright (C) 2026 Toit contributors.
"""Recalibrate saved scan projects; replay exact common-language scores.

Replaying requires identical contexts and candidate surfaces. Both binaries
recalibrate the same cached OCR; the old result must reproduce the saved physics.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from evaluate_span_ranking import rank
from PIL import Image


def load(path):
    return json.loads(Path(path).read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value,indent=2)+'\n')


def replay(reference):
    cached = load(reference)
    request = json.load(sys.stdin)
    assert request['schema_version']==1 and request['task']=='score_context_candidates'
    assert request['contexts']==cached['request']['contexts'], 'Cached context mismatch'
    originals = {(r['id'],r['target']):r['text'] for r in cached['request']['items']}
    scores = {(r['id'],r['target']):r for r in cached['scores']}
    result=[]
    seen=set()
    for item in request['items']:
        key=(item['id'],item['target'])
        assert key not in seen and originals[key]==item['text'], 'Cached candidate mismatch'
        seen.add(key)
        result.append({k:scores[key][k] for k in ['id','target','token_log_score_sum','token_count']})
    print(json.dumps(dict(schema_version=1,scores=result,
        method='exact_replay_of_common_language_control',reference_sha256=digest(reference))))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--replay',type=Path)
    p.add_argument('--old-binary',type=Path)
    p.add_argument('--new-binary',type=Path)
    p.add_argument('--manifest',type=Path)
    p.add_argument('--common-language',type=Path)
    p.add_argument('--catalog',type=Path)
    p.add_argument('--output',type=Path)
    a=p.parse_args()
    if a.replay:
        replay(a.replay)
        return
    if any(getattr(a,k) is None for k in ['old_binary','new_binary','manifest','common_language','catalog','output']):
        p.error('Supply binaries, manifest, common-language directory, catalog and output')
    a.output.mkdir(parents=True,exist_ok=False)
    cases=[c for c in load(a.manifest)['cases'] if c['track']=='closed']
    references={}
    for path in a.common_language.glob('case-*.json'):
        row=load(path)
        references[row['cohort'],row['case']]=path
    assert len(cases)==len(references)==31
    catalog=load(a.catalog)
    for font in catalog:
        assert digest(font['path'])==font['sha256']
    frozen={str(path):digest(path) for path in [Path(__file__),Path(__file__).with_name('evaluate_span_ranking.py'),
        a.manifest,a.catalog,a.old_binary,a.new_binary,*references.values()]}
    for case in cases:
        for path in [Path(case['project'])/'project.json',Path(case['control']),Path(case['candidates'])]:
            frozen[str(path)]=digest(path)
        project=load(Path(case['project'])/'project.json')
        for page in project['pages']:
            image=Path(case['project'])/page['image']
            with Image.open(image) as raster:
                assert raster.mode=='L' and raster.size==(page['width'],page['height'])
                assert hashlib.sha256(raster.tobytes()).hexdigest()==page['image_sha256']
            frozen[str(image)]=digest(image)
    write(a.output/'frozen.json',dict(files=frozen,cases=cases,
        scope='31 reused controlled masks. Cached OCR and exact cached T5 scores; replay validates context/surfaces. No new OCR or model inference. Old physics must reproduce saved control.'))
    results=[]
    for i,case in enumerate(cases):
        directory=a.output/f'case-{i:03}'
        directory.mkdir()
        source=load(Path(case['project'])/'project.json')
        reference=references[case['cohort'],case['id']]
        expected=load(case['control'])
        row=dict(case=case['id'],cohort=case['cohort'],modes={})
        for mode,binary in [('old',a.old_binary),('bounded',a.new_binary)]:
            project=directory/mode
            project.mkdir()
            data=copy.deepcopy(source)
            for page in data['pages']:
                page['image']=str((Path(case['project'])/page['image']).resolve())
            write(project/'project.json',data)
            errors=[]
            for profile in source['profiles']:
                assert all(f['width_metric']=='ink' and f['measurement_mode']=={'kind':'design'}
                           and not f.get('spacing') for f in profile['fits'])
                assert not profile.get('spacing')
                assert not profile.get('typography_group') and not profile.get('neural_search')
                cmd=[str(binary.resolve()),'calibrate','--project',str(project),
                     '--page',profile['page'],'--id',profile['id']]
                if profile['block']:
                    cmd+=['--block',profile['block']]
                for font in catalog:
                    cmd+=['--font',font['path']]
                proc=subprocess.run(cmd,capture_output=True,text=True,timeout=120)
                (project/f"{profile['id']}-calibrate.stderr").write_text(proc.stderr)
                if proc.returncode:
                    errors.append(proc.stderr)
            if errors:
                row['modes'][mode]=dict(status='error',errors=errors)
                continue
            cmd=[str(binary.resolve()),'context-rank','--project',str(project),
                 '--group','comparison','--candidates',case['candidates'],
                 '--paragraph-context','--sequence-score','--provider',sys.executable,
                 '--provider-arg',str(Path(__file__).resolve()),'--provider-arg=--replay',
                 '--provider-arg',str(reference.resolve()),'--output',str(project/'ranking.json')]
            proc=subprocess.run(cmd,capture_output=True,text=True,timeout=120)
            (project/'context.stderr').write_text(proc.stderr)
            if proc.returncode:
                raise RuntimeError(proc.stderr)
            scored=load(project/'ranking.json')
            assert scored['contexts']==expected['contexts']
            if mode=='old':
                assert scored['physical']==expected['physical'], 'Old cached physics does not reproduce'
                assert rank(scored['ranked_groups'],case['truth_ids'])==load(reference)['results']['local_geometry']['truth_rank']
            row['modes'][mode]=dict(status='ok',truth_rank=rank(scored['ranked_groups'],case['truth_ids']),
                compatible_candidates=scored['physical']['compatible_candidates'],
                physical_identical_to_control=scored['physical']==expected['physical'],
                truth_compatible=any(c['id'] in case['truth_ids'] and c['compatible'] for c in scored['physical']['candidates']))
        results.append(row)
        write(directory/'evaluation.json',row)
        print(json.dumps(row),flush=True)
    assert all(digest(path)==sha for path,sha in frozen.items())
    write(a.output/'summary.json',dict(cases=results,all_frozen_hashes_verified=True))


if __name__=='__main__':
    main()
