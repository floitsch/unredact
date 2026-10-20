# Copyright (C) 2026 Toit contributors.
"""Check natural candidate provenance and balanced paired lists independently."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path


def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--benchmark',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    root=a.benchmark;provenance=json.loads((root/'vocabulary-provenance.json').read_text());parent=Path(provenance['parent'])
    pool=json.loads((root/'visible-candidate-pool.json').read_text());labels=json.loads((root/'controller/labels.json').read_text())['labels']
    source_labels=json.loads((parent/'controller/labels.json').read_text())['labels']
    assert labels==source_labels
    assert digest(parent/'manifest.json')==provenance['parent_manifest_sha256']
    assert digest(root/'visible-candidate-pool.json')==provenance['visible_pool_sha256']
    for file,expected in provenance['source_inputs'].items():assert digest(file)==expected
    source_pages={}
    for label in labels:
        source_pages.setdefault((label['document'],label['page']),[]).append(label)
    ocr_cache={};origin_count=0
    for text,origins in pool['origins'].items():
        assert origins
        for origin in origins:
            group=source_pages[(origin['document'],origin['page'])]
            assert all(label['split']=='development' for label in group)
            assert origin['pixel_sha256']==group[0]['original_pixel_sha256']
            path=parent/'controller'/f"{origin['document']}-p{origin['page']}-ocr.json"
            assert str(path) in provenance['source_inputs']
            if path not in ocr_cache:ocr_cache[path]=json.loads(path.read_text())
            ocr=ocr_cache[path];indices=origin['indices']
            assert text==' '.join(ocr['text'][i] for i in indices)
            assert all(85<=float(ocr['conf'][i])<=100 for i in indices)
            assert len({tuple(ocr[k][i] for k in ('block_num','par_num','line_num')) for i in indices})==1
            b=origin['box']
            for label in group:
                m=label['box'];assert not(b[0]<m[2] and m[0]<b[2] and b[1]<m[3] and m[1]<b[3])
                assert not(set(indices)&set(label['indices']))
            origin_count+=1
    draws=json.loads((root/'controller/candidate-draws.json').read_text())
    for label in labels:
        d=draws[label['id']]
        assert len(d['natural'])==len(set(d['natural']))==300
        assert all(text in pool['origins'] for text in d['natural'])
        assert label['text'] in d['closed'] and len(d['closed'])==len(set(d['closed']))==300
        for kind in ['natural','closed']:assert Counter(len(t.split()) for t in d[kind])=={1:150,2:150}
        assert len(set(d['closed'])-set(d['natural']))<=1
    manifest=json.loads((root/'manifest.json').read_text());bycase={c['id']:(r,c) for r in labels for c in r['cases']}
    checked=0
    for name,info in manifest['suites'].items():
        path=root/(name+'.json');assert digest(path)==info['sha256'];suite=json.loads(path.read_text())
        original=json.loads((parent/(name+'.json')).read_text())
        for old,case in zip(original['cases'],suite['cases'],strict=True):
            row,meta=bycase[case['id']]
            assert case['id']==old['id'] and case['truth_id']==old['truth_id']
            assert {k:v for k,v in case['input'].items() if k!='candidates'}=={k:v for k,v in old['input'].items() if k!='candidates'}
            wanted=draws[row['id']]['closed']
            if meta['availability']=='absent':wanted=[t for t in wanted if t!=row['text']]
            assert [c['canonical_name'] for c in case['input']['candidates']]==wanted
            checked+=1
    result=dict(visible_candidates=len(pool['origins']),verified_origins=origin_count,development_source_pages=len(ocr_cache),cases_checked=checked,
                spans=len(labels),all_images_targets_labels_and_splits_unchanged=True,test_ocr_sources=0,
                interpretation='All provenance origins outside selected masks on development pages. Balanced candidate lists; closed answer injection explicit. No scoring or model outputs used.')
    with a.output.open('x') as f:json.dump(result,f,indent=2);f.write('\n')
    print(json.dumps(result,indent=2))


if __name__=='__main__':main()
