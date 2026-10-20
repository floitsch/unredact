# Copyright (C) 2026 Toit contributors.
"""Evaluate training-image font suggestions with a full-font-catalog control."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import subprocess


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def error(rows,key):
    return math.sqrt(sum((r[key]-r['observed'])**2 for r in rows)/len(rows)) if rows else None


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--projects',type=Path,nargs='+',required=True)
    p.add_argument('--config',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--binary',type=Path,default=Path('target/release/examples/audit_neural'))
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=False)
    artifacts=[a.binary,a.config,Path(__file__),Path('examples/audit_neural.rs'),Path('src/neural.rs'),
               Path('scripts/neural/infer.py'),Path('scripts/neural/typography.py')]
    config=json.loads(a.config.read_text())
    artifacts += [(a.config.parent/Path(config[k])).resolve() for k in ('model','catalog')]
    provenance={str(p):sha(p) for p in artifacts}
    provenance.update({str(p/'project.json'):sha(p/'project.json') for p in a.projects})
    (a.output/'provenance.json').write_text(json.dumps(provenance,indent=2)+'\n')
    rows,results=[],[]
    for i,source in enumerate(a.projects):
        # Keep source imports unchanged; neural montage artifacts go into the copy.
        project=a.output/f'case-{i}-project';shutil.copytree(source,project)
        run=subprocess.run([str(a.binary.resolve()),str(project),str(a.config)],capture_output=True,text=True,timeout=600)
        (a.output/f'case-{i}.stderr').write_text(run.stderr)
        if run.returncode:raise RuntimeError(run.stderr)
        (a.output/f'case-{i}.json').write_text(run.stdout)
        groups=[g for page in json.loads(run.stdout)['pages'] for g in page['groups']]
        observations=[r for g in groups if g['status']=='ok' for r in g['audit']]
        row=dict(project=str(source),audit_words=len(observations),
            **{kind+'_rmse':error(observations,kind+'_prediction') for kind in ('default','neural','catalog')},
            failures=[g for g in groups if g['status']!='ok'],
            selections=[{kind:g[kind]['fits'][0]['font'] for kind in ('default','neural','catalog')}
                        for g in groups if g['status']=='ok'])
        rows.extend(observations);results.append(row);print(json.dumps(row),flush=True)
    assert all(sha(p)==h for p,h in provenance.items())
    summary=dict(cases=results,audit_words=len(rows),
        **{kind+'_rmse':error(rows,kind+'_prediction') for kind in ('default','neural','catalog')},
        interpretation='Paired audit widths, not original-redaction recovery. Full-catalog control separates font expansion from neural selection. Failed/sparse regions excluded and listed. Within-page observations correlated, OCR uncorrected.')
    (a.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')


if __name__=='__main__':main()
