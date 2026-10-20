# Copyright (C) 2026 Toit contributors.
"""Measure existing grid detection on all development crops, without text scoring.

The CLI receives only an opaque crop path and a fixed search range. Expected
grid/condition/document metadata stays in this controller. Variants are paired,
not independent samples. This diagnoses detection, not hidden-text recovery.
"""
import argparse
from collections import defaultdict
import hashlib
import json
import math
from pathlib import Path
import subprocess

from PIL import Image


CONDITIONS = ['opaque', 'opaque-low-resolution', 'opaque-jpeg-noise',
              'opaque-subpixel', 'pixelate-8', 'pixelate-16', 'ordinary-text']


def load(path):
    return json.loads(Path(path).read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--benchmark', type=Path, default=Path('runs/multidoc-blinded-001'))
    p.add_argument('--original', type=Path, default=Path('runs/multidoc-benchmark-001'))
    p.add_argument('--binary', type=Path, default=Path('target/release/unredact'))
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    labels_path = args.benchmark / 'controller/labels.json'
    labels = [r for r in load(labels_path)['labels'] if r['split'] == 'development']
    if len(labels) != 96 or len({r['document'] for r in labels}) != 4:
        raise ValueError('Expected full four-document, 96-span development cohort')
    manifest = load(args.benchmark / 'manifest.json')
    if manifest['label_review'] != 'verified':
        raise ValueError('Unreviewed labels')
    cases = {}
    sources = [Path(__file__), args.binary, labels_path, args.benchmark / 'manifest.json']
    for condition in CONDITIONS[:-1]:
        path = args.benchmark / f'development-{condition}-present.json'
        if digest(path) != manifest['suites'][path.stem]['sha256']:
            raise ValueError('Suite changed')
        cases[condition] = {c['id']: c for c in load(path)['cases']}
        sources.append(path)
    tasks = []
    for label in labels:
        for condition in CONDITIONS:
            identity = hashlib.sha256((label['id'] + ':' + condition).encode()).hexdigest()
            if condition == 'ordinary-text':
                path = args.original / 'controller' / f"{label['document']}-p{label['page']}.pgm"
                box = label['box']
                pixel_hash = label['original_pixel_sha256']
            else:
                case_id = next(c['id'] for c in label['cases'] if c['condition'] == condition and c['availability'] == 'present')
                case = cases[condition][case_id]
                target = case['input']['targets'][0]
                b = target['bounds']
                box = [b['x'], b['y'], b['x'] + b['w'], b['y'] + b['h']]
                # Match Rust GrayImage::crop for the fractional-shift control.
                box = [math.floor(box[0]), math.floor(box[1]), math.ceil(box[2]), math.ceil(box[3])]
                page = next(p for p in case['input']['pages'] if p['id'] == target['page'])
                path = args.benchmark / page['image']
                if digest(path) != page['image_sha256']:
                    raise ValueError('Image changed')
                pixel_hash = None
            sources.append(path)
            tasks.append(dict(id=identity, path=str(path.resolve()), box=list(map(int, box)), pixel_hash=pixel_hash,
                              span=label['id'], document=label['document'], kind=label['kind'], condition=condition))
    frozen = {str(p.resolve()): digest(p) for p in sources}
    (out / 'plan.json').write_text(json.dumps(dict(tasks=tasks, files=frozen, min_block=4, max_block=20, keep=3,
        interpretation='All96 development spans, seven paired conditions. Grid diagnosis only. Test documents not evaluated.'), indent=2) + '\n')
    rows = []
    for task in tasks:
        image = Image.open(task['path']).convert('L')
        if task['pixel_hash'] and hashlib.sha256(image.tobytes()).hexdigest() != task['pixel_hash']:
            raise ValueError('Original pixel hash mismatch')
        crop = image.crop(task['box'])
        path = out / (task['id'] + '.pgm')
        crop.save(path)
        report_path = path.with_suffix('.json')
        result = subprocess.run([str(args.binary.resolve()), 'analyze-pixels', '--image', str(path),
                                 '--min-block', '4', '--max-block', '20', '--keep', '3', '--output', str(report_path)],
                                capture_output=True, text=True, timeout=30)
        row = {k: task[k] for k in ['id', 'span', 'document', 'kind', 'condition']}
        row.update(width=crop.width, height=crop.height, returncode=result.returncode, error=result.stderr)
        if result.returncode == 0:
            report = load(report_path)
            grids = [h['grid'] for h in report['hypotheses']]
            expected = None
            if task['condition'].startswith('pixelate-'):
                block = int(task['condition'].split('-')[1])
                expected = dict(block=block, phase_x=task['box'][0] % block, phase_y=task['box'][1] % block)
            row.update(status=report['status'], grids=grids, detected=bool(grids), expected=expected,
                       exact_first=expected is not None and bool(grids) and expected == grids[0],
                       exact_retained=expected is not None and expected in grids,
                       too_short_for_three_boundaries=expected is not None and crop.height - 1 < 3 * expected['block'])
        rows.append(row)
    for path, expected in frozen.items():
        if digest(path) != expected:
            raise ValueError('Frozen input/runtime changed')
    buckets = defaultdict(list)
    for row in rows:
        for kind in ['all', row['kind']]:
            buckets[row['condition'], kind].append(row)
    summary = [dict(condition=c, kind=k, cases=len(v), failures=sum(r['returncode'] != 0 for r in v),
                    detected=sum(r.get('detected', False) for r in v),
                    exact_first=sum(r.get('exact_first', False) for r in v),
                    exact_retained=sum(r.get('exact_retained', False) for r in v),
                    too_short=sum(r.get('too_short_for_three_boundaries', False) for r in v))
               for (c, k), v in sorted(buckets.items())]
    report = dict(rows=rows, summary=summary, frozen_verified=True,
                  interpretation='Detection metrics only. A real grid may result from ordinary resampling, so grid detection alone does not prove concealment. Variants share spans and source documents.')
    (out / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    for row in summary:
        if row['kind'] == 'all':
            print(json.dumps(row))


if __name__ == '__main__':
    main()
