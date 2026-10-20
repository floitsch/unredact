# Copyright (C) 2026 Toit contributors.
"""Audit fixture identity, paired coverage, document splits and input boundaries."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path

from PIL import Image


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def audit(root):
    manifest=json.loads((root/'manifest.json').read_text())
    plan=json.loads((root/'plan.json').read_text())
    labels=json.loads((root/'controller/labels.json').read_text())['labels']
    if len({r['id'] for r in labels})!=len(labels):raise ValueError('Duplicate span')
    index={c['id']:(r,c) for r in labels for c in r['cases']}
    expected=len(labels)*len(plan['conditions'])*2
    if len(index)!=expected or manifest['cases']!=expected:raise ValueError('Incomplete paired case generation')
    document_groups={d['id']:d['group'] for d in plan['documents']}
    splits={};pixels={};files={};seen=set();image_counts=Counter()
    for row in labels:
        group=document_groups[row['document']]
        if row['source_group']!=group or splits.setdefault(group,row['split'])!=row['split']:
            raise ValueError('Source group crosses splits')
        old=pixels.setdefault(row['original_pixel_sha256'],(row['document'],row['page']))
        if old!=(row['document'],row['page']):raise ValueError('Duplicate original page')
    for name,suite_info in manifest['suites'].items():
        path=root/(name+'.json')
        if digest(path)!=suite_info['sha256']:raise ValueError('Suite changed')
        suite=json.loads(path.read_text())
        if len(suite['cases'])!=suite_info['cases']:raise ValueError('Wrong case count')
        for case in suite['cases']:
            if case['id'] in seen:raise ValueError('Duplicate case')
            seen.add(case['id']);row,meta=index[case['id']]
            if meta['suite']!=name:raise ValueError('Incorrect stratum')
            data=case['input']
            if set(data)!={'schema_version','pages','targets','candidates'}:raise ValueError('Unexpected input metadata')
            candidates=data['candidates'];ids=[c['id'] for c in candidates]
            truth=hashlib.sha256(('candidate:'+row['text']).encode()).hexdigest()
            if case['truth_id']!=truth or len(ids)!=len(set(ids)):raise ValueError('Truth/ID mismatch')
            present=meta['availability']=='present'
            if (truth in ids)!=present or len(ids)!=plan['candidate_count']-(not present):raise ValueError('Candidate availability mismatch')
            if present and next(c for c in candidates if c['id']==truth)['canonical_name']!=row['text']:
                raise ValueError('Candidate label mismatch')
            for target in data['targets']:
                if target['left_context'] or target['right_context']:raise ValueError('Supplied text context')
                kind=target['evidence']['kind']
                if kind!=('mosaic_auto' if meta['condition'].startswith('pixelate-') else 'opaque'):
                    raise ValueError('Wrong evidence kind')
            for page in data['pages']:
                image_path=(root/page['image']).resolve()
                if not image_path.is_relative_to((root/'images').resolve()):raise ValueError('Image outside input directory')
                if str(image_path) not in files:
                    if digest(image_path)!=page['image_sha256']:raise ValueError('Image changed')
                    with Image.open(image_path) as image:
                        if image.format!='PPM' or image.mode!='L':raise ValueError('Expected flattened grayscale raster')
                        files[str(image_path)]=dict(sha256=page['image_sha256'],size=image.size)
                    image_counts[meta['condition']]+=1
                if files[str(image_path)]['sha256']!=page['image_sha256']:raise ValueError('Inconsistent repeated page hash')
                width,height=files[str(image_path)]['size']
                if len(page['redaction_regions'])!=plan['spans_per_page']:raise ValueError('Other selected spans are not masked')
                for b in page['redaction_regions']:
                    if not(0<=b['x']<b['x']+b['w']<=width and 0<=b['y']<b['y']+b['h']<=height):
                        raise ValueError('Mask outside image')
    if seen!=set(index):raise ValueError('Missing cases')
    original_hashes={}
    for row in labels:
        path=root/'controller'/f"{row['document']}-p{row['page']}.pgm"
        if str(path) not in original_hashes:
            with Image.open(path) as image:original_hashes[str(path)]=hashlib.sha256(image.convert('L').tobytes()).hexdigest()
        if original_hashes[str(path)]!=row['original_pixel_sha256']:raise ValueError('Original page changed')
    return dict(schema_version=1,unique_documents=len({r['document'] for r in labels}),
                unique_original_pages=len(pixels),unique_spans=len(labels),paired_cases=len(seen),
                by_split=dict(Counter(r['split'] for r in labels)),by_kind=dict(Counter(r['kind'] for r in labels)),
                by_span_length=dict(Counter(r['count'] for r in labels)),images_by_condition=dict(image_counts),
                manifest_sha256=digest(root/'manifest.json'),labels_sha256=digest(root/'controller/labels.json'),
                label_review=manifest['label_review'],image_files=files,
                interpretation='Structural audit only; OCR transcriptions require independent visual review. No solver outcomes inspected.')


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--benchmark',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();result=audit(a.benchmark)
    with a.output.open('x') as f:json.dump(result,f,indent=2);f.write('\n')
    print(json.dumps({k:v for k,v in result.items() if k!='image_files'},indent=2))


if __name__=='__main__':main()
