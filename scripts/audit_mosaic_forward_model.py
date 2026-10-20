# Copyright (C) 2026 Toit contributors.
"""Controller-only validation of partial-cell physics after a complete evaluation.

Original hidden pixels are used exclusively as an oracle here, never as scorer
inputs. The oracle checks the forward model's assumptions/coordinates separately
from typography and candidate ranking. It is not text-recovery performance.
"""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import statistics

import numpy as np
from PIL import Image


def load(path):
    return json.loads(Path(path).read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evaluation', type=Path, required=True)
    p.add_argument('--benchmark', type=Path, default=Path('runs/multidoc-blinded-coordinates-001'))
    p.add_argument('--original', type=Path, default=Path('runs/multidoc-benchmark-001'))
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    summary = load(a.evaluation / 'summary.json')
    if len(summary['rows']) != 384 or not summary['all_frozen_hashes_verified']:
        raise ValueError('Complete384-case comparison required')
    label_path = a.benchmark / 'controller/labels.json'
    labels = {c['id']: r for r in load(label_path)['labels'] for c in r['cases']}
    artifacts = [Path(__file__), a.evaluation / 'summary.json', label_path]
    images, rows = {}, []
    for row in summary['rows']:
        if row['availability'] != 'present':
            continue
        label = labels[row['case']]
        outcome = row['engines']['shared_cells']
        result = dict(case=row['case'], document=label['document'], kind=label['kind'], condition=row['condition'],
                      status=outcome['status'], truth_rank=outcome.get('truth_rank'))
        if outcome['status'] != 'ok':
            rows.append(result)
            continue
        directory = a.evaluation / row['case']
        artifacts += [directory / 'shared_cells.json', directory / 'request.json']
        report, request = load(directory / 'shared_cells.json'), load(directory / 'request.json')
        source = a.original / 'controller' / f"{label['document']}-p{label['page']}.pgm"
        artifacts.append(source)
        if str(source) not in images:
            image = np.array(Image.open(source).convert('L'))
            if hashlib.sha256(image.tobytes()).hexdigest() != label['original_pixel_sha256']:
                raise ValueError('Original label image changed')
            images[str(source)] = image
        image = images[str(source)]
        bounds = request['regions'][request['target_index']]
        x, y, w, h = [int(bounds[k]) for k in ['x', 'y', 'w', 'h']]
        if [x, y, x+w, y+h] != label['box']:
            raise ValueError('Target/controller mapping mismatch')
        mask = image[y:y+h, x:x+w]
        expected_block = int(row['condition'].split('-')[1])
        models = [m for index, m in report['cell_models'] if report['grid_analysis']['hypotheses'][index]['grid'] ==
                  dict(block=expected_block, phase_x=0, phase_y=0)]
        if len(models) != 1:
            raise ValueError('Oracle audit requires the correctly inferred grid, separately checked')
        model = models[0]
        residuals = []
        for cell in model['cells']:
            left, top, right, bottom = cell['bounds_in_target']
            hidden_darkness = float((255 - mask[top:bottom, left:right]).sum())
            predicted = 255. - (hidden_darkness + cell['outside_darkness']) / cell['full_area']
            residuals.append(predicted - cell['observed_mean'])
        result.update(oracle_rmse=float(np.sqrt(np.mean(np.square(residuals)))),
                      oracle_max_absolute_error=max(map(abs, residuals)),
                      cells=len(model['cells']), complete_cells=model['complete_cells'], excluded_cells=model['excluded_cells'])
        # Text labels join model candidates only after their frozen scores exist.
        truth_ids = {c['id'] for c in request['candidates'] if any(s['text'] == label['text'] for s in c['surfaces'])}
        if len(truth_ids) != 1:
            raise ValueError('Expected one supplied true surface in present control')
        truth = next(c for c in report['scores'] if c['id'] in truth_ids)
        result.update(truth_renderable=truth['cost'] is not None, truth_rmse=truth['rmse'], truth_fit_index=truth['fit_index'])
        rows.append(result)
    if len(rows) != 192:
        raise ValueError('Expected all192 present-condition records')
    frozen = {str(path.resolve()): digest(path) for path in artifacts}
    buckets = defaultdict(list)
    for row in rows:
        for kind in ['all', row['kind']]:
            buckets[row['condition'], kind].append(row)
    metrics = []
    for (condition, kind), values in sorted(buckets.items()):
        valid = [v for v in values if v['status'] == 'ok']
        rendered = [v for v in valid if v['truth_renderable']]
        metrics.append(dict(condition=condition, kind=kind, cases=len(values), successful=len(valid),
            oracle_within_rounding=sum(v['oracle_max_absolute_error'] <= .5 + 1e-9 for v in valid),
            without_complete_cells=sum(v['complete_cells'] == 0 for v in valid),
            oracle_median_rmse=statistics.median(v['oracle_rmse'] for v in valid) if valid else None,
            truth_renderable=len(rendered), median_true_render_rmse=statistics.median(v['truth_rmse'] for v in rendered) if rendered else None))
    with a.output.open('x') as f:
        json.dump(dict(rows=rows, metrics=metrics, files=frozen,
            interpretation='Post-evaluation controller-only oracle. Original hidden pixels must reproduce measured cell means within8-bit rounding. This validates forward physics, not recoverability or font fitting; these original pixels never enter either scorer.'), f, indent=2)
        f.write('\n')
    for row in metrics:
        if row['kind'] == 'all':
            print(json.dumps(row))


if __name__ == '__main__':
    main()
