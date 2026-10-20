# Copyright (C) 2026 Toit contributors.
"""Evaluate OCR on manually reviewed visible neighbors, never hidden text."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--review', type=Path, required=True)
    p.add_argument('--tessdata', type=Path, action='append', required=True)
    p.add_argument('--psm', type=int, nargs='+', default=[3])
    p.add_argument('--oem', type=int, default=1)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    out = a.output.resolve(); out.mkdir(parents=True, exist_ok=False)
    review = json.loads(a.review.read_text())
    image = (a.review.parent / review['image']).resolve()
    rows = []
    for model_index, directory in enumerate(a.tessdata):
        for mode in a.psm:
            command = ['tesseract', str(image), 'stdout', '--psm', str(mode), '--oem', str(a.oem),
                       '--tessdata-dir', str(directory.resolve()), '-l', 'eng',
                       '-c', 'tessedit_create_tsv=1']
            start = time.monotonic()
            run = subprocess.run(command, capture_output=True, text=True, timeout=120, check=True)
            if not run.stdout.startswith('level\tpage_num\t'):
                raise ValueError(f'Tesseract did not produce TSV: {run.stderr}')
            elapsed = time.monotonic() - start
            (out / f'model-{model_index}-psm-{mode}.tsv').write_text(run.stdout)
            words = []
            for line in run.stdout.splitlines()[1:]:
                c = line.split('\t', 11)
                if len(c) != 12 or c[0] != '5' or not c[11].strip():
                    continue
                words.append(dict(text=c[11], confidence=float(c[10]), x=int(c[6]),
                                  y=int(c[7]), w=int(c[8]), h=int(c[9])))
            observations = []
            for target in review['targets']:
                b = target['bounds']
                line = [w for w in words if min(w['y']+w['h'], b['y']+b['h'])
                        - max(w['y'], b['y']) >= min(w['h'], b['h'])*.5]
                left = max((w for w in line if w['x']+w['w'] <= b['x']),
                           key=lambda w: w['x']+w['w'], default=None)
                right = min((w for w in line if w['x'] >= b['x']+b['w']),
                            key=lambda w: w['x'], default=None)
                for side, word in [('left', left), ('right', right)]:
                    observations.append(dict(target=target['id'], side=side, expected=target[side],
                        word=word, exact=word is not None and word['text'] == target[side],
                        accepted=word is not None and word['confidence'] >= 85))
            rows.append(dict(tessdata=str(directory.resolve()), model_sha256=digest(directory/'eng.traineddata'),
                psm=mode, oem=a.oem, command=command, stderr=run.stderr, elapsed_seconds=elapsed, observations=observations))
    report = dict(review_sha256=digest(a.review), image_sha256=digest(image),
        script_sha256=digest(Path(__file__)),
        tesseract_version=subprocess.check_output(['tesseract', '--version'], text=True), rows=rows,
        interpretation='Visible neighbors reviewed manually; development diagnostic, not a held-out recognition benchmark.')
    (out/'results.json').write_text(json.dumps(report, indent=2)+'\n')
    for r in rows:
        obs=r['observations']; print(r['tessdata'], r['psm'], 'exact',sum(o['exact'] for o in obs),
            '/',len(obs),'accepted correct',sum(o['exact'] and o['accepted'] for o in obs),
            'accepted wrong',sum(not o['exact'] and o['accepted'] for o in obs),flush=True)


if __name__ == '__main__': main()
