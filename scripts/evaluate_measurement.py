# Copyright (C) 2026 Toit contributors.
"""Paired audit of design and hinted metrics on existing scan imports."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def rmse(rows, key):
    return math.sqrt(sum((r[key]-r['observed'])**2 for r in rows)/len(rows)) if rows else None


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--projects', type=Path, nargs='+', required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--ppem', default='12,16,24,48,96,192')
    p.add_argument('--binary', type=Path, default=Path('target/release/examples/audit_measurement'))
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    binary_hash = sha(a.binary)
    source_hashes = {str(p): sha(p) for p in [Path(__file__), Path('examples/audit_measurement.rs'),
        Path('src/calibration.rs'), Path('src/native.rs'), Path('native/bridge.c')]}
    provenance = dict(binary_sha256=binary_hash, source_sha256=source_hashes, ppem=a.ppem,
                      projects={str(p):sha(p/'project.json') for p in a.projects})
    (a.output/'provenance.json').write_text(json.dumps(provenance,indent=2)+'\n')
    observations, results = [], []
    for i, project in enumerate(a.projects):
        run = subprocess.run([str(a.binary.resolve()),str(project),a.ppem], capture_output=True,
                             text=True,timeout=600)
        (a.output/f'case-{i}.stderr').write_text(run.stderr)
        if run.returncode: raise RuntimeError(run.stderr)
        (a.output/f'case-{i}.json').write_text(run.stdout)
        report = json.loads(run.stdout)
        groups = [g for page in report['pages'] for g in page['groups']]
        rows = [r for g in groups if g['status']=='ok' for r in g['audit']]
        result = dict(project=str(project), audit_words=len(rows), design_rmse=rmse(rows,'design_prediction'),
            expanded_rmse=rmse(rows,'expanded_prediction'), failures=[g for g in groups if g['status']!='ok'],
            modes=[g['expanded']['fits'][0]['measurement_mode'] for g in groups if g['status']=='ok'])
        observations.extend(rows);results.append(result)
        print(json.dumps(result),flush=True)
    assert sha(a.binary)==binary_hash
    assert all(sha(Path(p)/'project.json')==h for p,h in provenance['projects'].items())
    summary=dict(cases=results,audit_words=len(observations),design_rmse=rmse(observations,'design_prediction'),
        expanded_rmse=rmse(observations,'expanded_prediction'),
        interpretation='Paired cached-OCR width diagnostic. Correlated within-page observations and uncorrected OCR labels. Modes selected by validation only; expanded selection can overfit. No redaction recovery or source renderer identification claim.')
    (a.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')


if __name__ == '__main__':
    main()
