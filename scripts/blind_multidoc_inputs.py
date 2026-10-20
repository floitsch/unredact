# Copyright (C) 2026 Toit contributors.
"""Remove fixture metadata cues before language/pixelation evaluation.

Opaque identifiers and content-addressed regular image files replace descriptive
IDs/paths. Present and absent controls both have300 candidates, balanced150/150.
Private mappings retain exact correspondence to the original fixtures.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re


def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def key(kind,value):return kind+'-'+hashlib.sha256((kind+':'+value).encode()).hexdigest()
def write(path,value):path.write_text(json.dumps(value,indent=2)+'\n')


def replace_absent(candidates,truth,pool,span_id):
    """Fill the removed answer's class without exposing answer presence in size."""
    present={c['canonical_name'] for c in candidates}
    assert truth not in present
    count=len(truth.split())
    eligible=[t for t in pool if t not in present and t!=truth and len(t.split())==count]
    if not eligible:raise ValueError('No independent same-class distractor')
    chosen=min(eligible,key=lambda t:key('replacement',span_id+':'+t))
    replacement=dict(id=hashlib.sha256(('candidate:'+chosen).encode()).hexdigest(),canonical_name=chosen,
                     surfaces=[dict(text=chosen,kind='canonical')],source_ids=[],context_score=None)
    return sorted(candidates+[replacement],key=lambda c:c['canonical_name']),replacement


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--benchmark',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    source=a.benchmark.resolve();out=a.output.resolve()
    manifest=json.loads((source/'manifest.json').read_text());labels=json.loads((source/'controller/labels.json').read_text());plan=json.loads((source/'plan.json').read_text())
    assert manifest['label_review']=='verified' and plan['candidate_count']==300
    pool=json.loads((source/'visible-candidate-pool.json').read_text())['origins']
    metadata={c['id']:(row,c) for row in labels['labels'] for c in row['cases']}
    out.mkdir(parents=True,exist_ok=False);(out/'images').mkdir();(out/'controller').mkdir()
    mappings={};image_hashes={};cases=0
    for name,info in manifest['suites'].items():
        path=source/(name+'.json');assert digest(path)==info['sha256']
        suite=json.loads(path.read_text())
        for case in suite['cases']:
            old=copy.deepcopy(case);row,meta=metadata[old['id']];data=case['input'];pages={}
            for page in data['pages']:
                original=(source/page['image']).resolve();h=page['image_sha256']
                if h not in image_hashes:
                    assert digest(original)==h
                    dest=out/'images'/(h+'.pgm');os.link(original,dest)
                    assert not dest.is_symlink() and digest(dest)==h
                    image_hashes[h]=str(original)
                pages[page['id']]=key('p',page['id']);page['id']=pages[page['id']]
                page['image']='images/'+h+'.pgm'
            targets={}
            for target in data['targets']:
                targets[target['id']]=key('t',target['id']);target['id']=targets[target['id']];target['page']=pages[target['page']]
            replacement=None
            if meta['availability']=='absent':
                data['candidates'],replacement=replace_absent(data['candidates'],row['text'],pool,row['id'])
            assert len(data['candidates'])==len({c['id'] for c in data['candidates']})==300
            assert [sum(len(c['canonical_name'].split())==n for c in data['candidates']) for n in (1,2)]==[150,150]
            assert (case['truth_id'] in {c['id'] for c in data['candidates']})==(meta['availability']=='present')
            case['id']=key('c',case['id'])
            # Reverse every allowed change and require exact equality to parent.
            reverse=copy.deepcopy(case);reverse['id']=old['id']
            for page,before in zip(reverse['input']['pages'],old['input']['pages'],strict=True):
                page['id']=before['id'];page['image']=before['image']
            for target,before in zip(reverse['input']['targets'],old['input']['targets'],strict=True):
                target['id']=before['id'];target['page']=before['page']
            if replacement:reverse['input']['candidates']=[c for c in reverse['input']['candidates'] if c['id']!=replacement['id']]
            assert reverse==old
            assert all(re.fullmatch('t-[a-f0-9]{64}',t['id']) for t in data['targets'])
            mappings[old['id']]=dict(case=case['id'],target_ids=targets,page_ids=pages,replacement=replacement)
            cases+=1
        # Prevent fixed first-four-phrases/last-four-words order from exposing class.
        suite['cases'].sort(key=lambda c:c['id'])
        suite['description']='Opaque adapter IDs/image paths; natural visible-development candidate bank;300 candidates in both present/absent controls. Original masks and pixels unchanged; private controller mapping preserves labels.'
        write(out/(name+'.json'),suite);info['sha256']=digest(out/(name+'.json'))
    for row in labels['labels']:
        for case in row['cases']:case['id']=mappings[case['id']]['case']
    write(out/'controller/labels.json',labels);write(out/'controller/input-map.json',mappings)
    plan.update(adapter_identifiers='opaque SHA256 aliases',absent_candidate_count=300,absent_replacement='one same-class visible development distractor',case_order='opaque hash order within each suite')
    write(out/'plan.json',plan)
    manifest.update(parent=str(source),parent_manifest_sha256=digest(source/'manifest.json'),plan_sha256=digest(out/'plan.json'),
                    input_alias_generator_sha256=digest(__file__),equal_size_absent_controls=True)
    write(out/'manifest.json',manifest)
    write(out/'audit.json',dict(cases=cases,images=len(image_hashes),spans=len(labels['labels']),parent_manifest_sha256=digest(source/'manifest.json'),
          pixels_masks_evidence_and_labels_unchanged=True,exact_parent_input_reconstruction=True,
          fixed_candidate_count=300,fixed_word_phrase_counts=[150,150],regular_image_files=True,opaque_ids=True,
          private_mapping_sha256=digest(out/'controller/input-map.json'),image_sources=image_hashes,
          limitations='Protocol blinding, not filesystem sandboxing. Controller suites still know labels/conditions. Keep evaluation output directories opaque too. Original geometry-only runs did not consume encoded metadata; do not reuse their unblinded requests for language or pixelation claims.'))
    print('Created',cases,'blinded inputs from',len(labels['labels']),'spans; both availability controls have300 candidates')


if __name__=='__main__':main()
