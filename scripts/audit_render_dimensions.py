# Copyright (C) 2026 Toit contributors.
"""Diagnose candidate-render bounds on every completed development span.

The Rust diagnostic receives the complete candidate list and declared mask only.
Private truth labels select diagnostic rows after all dimensions are produced.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess


def load(path): return json.loads(Path(path).read_text())
def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--evaluation', type=Path, default=Path('runs/mosaic-cells-development-001/evaluation'))
    p.add_argument('--labels', type=Path, default=Path('runs/multidoc-blinded-coordinates-001/controller/labels.json'))
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    summary = load(a.evaluation / 'summary.json')
    cases = [r for r in summary['rows'] if r['condition'] == 'pixelate-8' and r['availability'] == 'present']
    if len(cases) != 96: raise ValueError('Full96 cases required')
    labels = {c['id']: r for r in load(a.labels)['labels'] for c in r['cases']}
    a.output.mkdir(parents=True, exist_ok=False)
    files = [Path(__file__), a.binary, a.labels, a.evaluation / 'summary.json',
             *[a.evaluation/r['case']/'request.json' for r in cases]]
    frozen = {str(p.resolve()): digest(p) for p in files}
    (a.output/'freeze.json').write_text(json.dumps(frozen,indent=2)+'\n')
    rows = []
    for case in cases:
        request_path = a.evaluation/case['case']/'request.json'
        request = load(request_path)
        result = subprocess.run([str(a.binary.resolve()), str(request_path.resolve())], capture_output=True, text=True, timeout=60)
        if result.returncode: raise RuntimeError(result.stderr)
        (a.output/(case['case']+'.json')).write_text(result.stdout)
        diagnostics = json.loads(result.stdout)
        label = labels[case['case']]
        truth = next(c['id'] for c in request['candidates'] if any(s['text']==label['text'] for s in c['surfaces']))
        fits = [d for d in diagnostics if d['id']==truth]
        can_fit = any(not r['clipped'] and r['width']<=d['target_width'] and r['height']<=d['target_height']
                      for d in fits for r in d['raster_bounds'])
        if can_fit: reason = 'renderable'
        elif all(d['raw_height']>d['target_height'] for d in fits): reason = 'all_fits_too_tall'
        elif all(d['raw_width']>d['target_width'] for d in fits): reason = 'all_fits_too_wide'
        else: reason = 'mixed_or_raster_bounds'
        prior = load(a.evaluation/case['case']/'shared_cells.json')
        supported = next(r['cost'] is not None for r in prior['scores'] if r['id']==truth)
        if supported != can_fit: raise ValueError('Diagnostic does not reproduce completed scorer support')
        rows.append(dict(case=case['case'],document=label['document'],page=label['page'],kind=label['kind'],
                         reason=reason,fits=fits))
    for path, sha in frozen.items():
        if digest(path)!=sha: raise ValueError('Frozen diagnostic dependency changed')
    report = dict(rows=rows,counts=dict(Counter(r['reason'] for r in rows)),
                  per_document={d:dict(Counter(r['reason'] for r in rows if r['document']==d)) for d in sorted({r['document'] for r in rows})},
                  frozen_verified=True)
    (a.output/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k: v for k,v in report.items() if k!='rows'},indent=2))


if __name__ == '__main__': main()
