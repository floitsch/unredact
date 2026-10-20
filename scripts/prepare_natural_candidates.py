# Copyright (C) 2026 Toit contributors.
"""Replace artificial distractors with visible development-document words/phrases.

The answer is injected only in the explicit closed-vocabulary track. Source
harvesting never reads test-page OCR or masked text. The images, locations,
labels, conditions and document split are inherited unchanged. No ranks are read.
"""
import argparse
from collections import defaultdict
import copy
import hashlib
import json
from pathlib import Path
import re
import shutil

import numpy as np


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def key(*values):
    return hashlib.sha256(':'.join(map(str,values)).encode()).hexdigest()


def write(path,value):
    path.write_text(json.dumps(value,indent=2)+'\n')


def intersects(a,b):
    return a[0]<b[2] and b[0]<a[2] and a[1]<b[3] and b[1]<a[3]


def visible_phrases(ocr,masks):
    """Bad words/masks remain barriers, including when OCR skipped the hidden word."""
    words=[]
    for i,text in enumerate(ocr['text']):
        if not text.strip():continue
        x,y,w,h=(int(ocr[k][i]) for k in ('left','top','width','height'))
        box=(x,y,x+w,y+h)
        good=bool(re.fullmatch('[A-Za-z]{3,18}',text)) and 85<=float(ocr['conf'][i])<=100
        good=good and w>0 and h>0 and not any(intersects(box,b) for b in masks)
        words.append(dict(index=i,text=text,box=box,good=good,
                          line=tuple(ocr[k][i] for k in ('block_num','par_num','line_num'))))
    for n in (1,2):
        for start in range(len(words)-n+1):
            group=words[start:start+n]
            if not all(w['good'] for w in group) or len({w['line'] for w in group})!=1:continue
            if n==2:
                a,b=[w['box'] for w in group];height=min(a[3]-a[1],b[3]-b[1])
                if not(0<=b[0]-a[2]<=2*height and min(a[3],b[3])-max(a[1],b[1])>=.5*height):continue
            box=(min(w['box'][0] for w in group),min(w['box'][1] for w in group),
                 max(w['box'][2] for w in group),max(w['box'][3] for w in group))
            if any(intersects(box,b) for b in masks):continue
            yield dict(text=' '.join(w['text'] for w in group),count=n,indices=[w['index'] for w in group],box=box)


def draw_vocabulary(pools,truth,count,seed):
    if count<2 or count%2:raise ValueError('Even positive candidate count required')
    rng=np.random.default_rng(seed);draw=[]
    for n in (1,2):
        population=sorted(pools[n])
        if len(population)<count//2:raise ValueError('Not enough distinct visible candidates')
        selected=[population[int(i)] for i in rng.choice(len(population),count//2,replace=False)]
        draw+=selected
    open_draw=sorted(draw)
    closed=draw[:]
    if truth not in closed:
        n=len(truth.split())
        if n not in (1,2):raise ValueError('Expected one/two-word truth')
        # A fixed balanced mixture avoids leaking the answer class through counts.
        index=int(rng.integers((n-1)*(count//2),n*(count//2)))
        closed[index]=truth
    return open_draw,sorted(closed)


def candidate(text):
    return dict(id=key('candidate',text),canonical_name=text,surfaces=[dict(text=text,kind='canonical')],
                source_ids=[],context_score=None)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--benchmark',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--seed',type=int,default=681741)
    args=p.parse_args();source=args.benchmark.resolve();out=args.output.resolve()
    manifest=json.loads((source/'manifest.json').read_text())
    if manifest['label_review']!='verified':raise ValueError('Source labels must be verified')
    labels=json.loads((source/'controller/labels.json').read_text());plan=json.loads((source/'plan.json').read_text())
    out.mkdir(parents=True,exist_ok=False);(out/'controller').mkdir()
    # Read-only references to original pixels; manifests still hash their bytes.
    (out/'images').symlink_to(source/'images',target_is_directory=True)
    for path in (source/'controller').iterdir():
        destination=out/'controller'/path.name
        if path.suffix=='.pgm':destination.symlink_to(path)
        elif path.name in ('labels.json','label-review.json'):shutil.copyfile(path,destination)
    pages=defaultdict(list)
    for label in labels['labels']:
        if label['split']=='development':pages[(label['document'],label['page'])].append(label)
    pools={1:set(),2:set()};origins=defaultdict(list);inputs={}
    for (doc,page),selected in sorted(pages.items()):
        ocr_path=source/'controller'/f'{doc}-p{page}-ocr.json'
        inputs[str(ocr_path)]=digest(ocr_path)
        masks=[r['box'] for r in selected]
        ocr=json.loads(ocr_path.read_text())
        for phrase in visible_phrases(ocr,masks):
            pools[phrase['count']].add(phrase['text'])
            origins[phrase['text']].append(dict(document=doc,page=page,pixel_sha256=selected[0]['original_pixel_sha256'],**phrase))
    write(out/'visible-candidate-pool.json',dict(schema_version=1,source_split='development',inputs=inputs,
          counts={n:len(v) for n,v in pools.items()},origins=dict(sorted(origins.items())),
          interpretation='Visible OCR spans only; mask intersections and low-confidence/invalid words are barriers. Test OCR is never read. Origins are provenance, not guarantees of OCR correctness. Repeated visible instances of a target string may be harvested.'))
    draws={};availability=defaultdict(lambda:dict(spans=0,found_in_pool=0,found_in_sample=0))
    for row in labels['labels']:
        natural,closed=draw_vocabulary(pools,row['text'],plan['candidate_count'],int(key(args.seed,row['id'])[:8],16))
        draws[row['id']]=dict(natural=natural,closed=closed)
        a=availability[row['split']];a['spans']+=1;a['found_in_pool']+=row['text'] in pools[row['count']];a['found_in_sample']+=row['text'] in natural
    bycase={c['id']:(row,c) for row in labels['labels'] for c in row['cases']}
    for name,entry in manifest['suites'].items():
        path=source/(name+'.json')
        if digest(path)!=entry['sha256']:raise ValueError('Source suite changed')
        suite=json.loads(path.read_text())
        for case in suite['cases']:
            row,meta=bycase[case['id']]
            texts=draws[row['id']]['closed']
            if meta['availability']=='absent':texts=[t for t in texts if t!=row['text']]
            case['input']['candidates']=[candidate(t) for t in texts]
        suite['description']='Same document-disjoint image benchmark; natural visible development-source distractors, balanced150single/150two-word closed list; answer explicitly injected for present cases and removed for absent controls. No open recovery claim.'
        write(out/(name+'.json'),suite);entry['sha256']=digest(out/(name+'.json'))
    # Untouched non-injected draws are stored separately, with their measured
    # availability; these are not silently substituted for closed-list results.
    write(out/'controller/candidate-draws.json',draws)
    write(out/'vocabulary-provenance.json',dict(schema_version=1,parent_manifest_sha256=digest(source/'manifest.json'),
          parent=str(source),generator_sha256=digest(__file__),seed=args.seed,source_inputs=inputs,
          visible_pool_sha256=digest(out/'visible-candidate-pool.json'),availability=dict(availability),
          policy='No model results read. Closed lists insert the answer, so availability is not discovery. Natural non-injected draws are separately recorded for future open-track evaluation. Fixed150/150 mixture; no length/geometry or semantic matching to hidden answer.'))
    plan['vocabulary']='Visible unmasked development OCR spans, balanced one-/two-word mixture; closed truth injection explicit'
    plan['vocabulary_seed']=args.seed
    write(out/'plan.json',plan)
    manifest.update(parent_manifest_sha256=digest(source/'manifest.json'),plan_sha256=digest(out/'plan.json'),
                    vocabulary_sha256=digest(out/'visible-candidate-pool.json'),vocabulary_generator_sha256=digest(__file__),
                    vocabulary_provenance_sha256=digest(out/'vocabulary-provenance.json'))
    write(out/'manifest.json',manifest)
    print(json.dumps(dict(unique_visible_candidates={n:len(p) for n,p in pools.items()},availability=dict(availability)),indent=2))


if __name__=='__main__':main()
