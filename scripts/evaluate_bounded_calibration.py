# Copyright (C) 2026 Toit contributors.
"""Paired calibration on independent Pillow rasters with supplied word labels.

This isolates width fitting. It does not evaluate OCR or unknown redactions.
All fixtures are prepared and hashed before either binary is executed.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import subprocess

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont
from evaluate_scans import TEXT


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def prepare(output, catalog):
    words = sorted(set(re.findall(r'\b[a-z]{3,}\b', TEXT.lower())))
    fonts = {f['id']: f for f in catalog}
    cases = []
    for family in ['LiberationSerif-Regular', 'Lato-Regular',
                   'LiberationMono-Regular', 'DejaVuSansMono']:
        for size in [17, 29]:
            for tracking in [-0.4, 1.1]:
                identity = f'{family}-{size}-{tracking}'
                directory = output / identity
                directory.mkdir()
                font = ImageFont.truetype(fonts[family]['path'], size=size)
                page = Image.new('L', (2400, ((len(words)+3)//4)*64), 255)
                observations = []
                rng = np.random.default_rng(44127)
                for i, word in enumerate(words):
                    patch = Image.new('L', (600, 64), 255)
                    draw = ImageDraw.Draw(patch)
                    # Render separate glyphs with pair-kerning-aware positions;
                    # disabling ligatures makes the extra spacing unambiguous.
                    for j, char in enumerate(word):
                        position = font.getlength(word[:j+1], features=['-liga']) - font.getlength(char)
                        draw.text((10 + position + j*tracking, 10), char, font=font, fill=0)
                    patch = patch.filter(ImageFilter.GaussianBlur(0.45))
                    pixels = np.clip(np.asarray(patch, dtype=float) + rng.normal(0, 1.8, (64,600)),0,255).astype('uint8')
                    yy, xx = np.where(pixels < 165)
                    assert len(xx) and xx.min() > 0 and xx.max() < 599
                    x, y = (i%4)*600, (i//4)*64
                    page.paste(Image.fromarray(pixels), (x,y))
                    observations.append(dict(text=word, confidence=99., block='body', line=f'line-{i}',
                        bounds=dict(x=float(x+xx.min()), y=float(y+yy.min()),
                                    w=float(xx.max()-xx.min()+1), h=float(yy.max()-yy.min()+1))))
                image = directory/'page.pgm'
                page.save(image)
                project = dict(schema_version=1, profiles=[], targets=[], pages=[dict(
                    id='p', source=str(image.resolve()), source_sha256=sha(image), page_number=0,
                    dpi=300., width=page.width, height=page.height, image=str(image.resolve()),
                    image_sha256=hashlib.sha256(page.tobytes()).hexdigest(), likely_scan=True, image_coverage=1., native_chars=[],
                    proposed_redactions=[], words=observations)])
                write(directory/'input.json', project)
                cases.append(dict(id=identity, font=family, size=size, tracking=tracking,
                    image_sha256=sha(image), project_sha256=sha(directory/'input.json')))
    return cases


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--old-binary', type=Path, required=True)
    p.add_argument('--new-binary', type=Path, required=True)
    p.add_argument('--catalog', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    catalog = json.loads(a.catalog.read_text())
    for font in catalog:
        assert sha(Path(font['path'])) == font['sha256']
    cases = prepare(a.output, catalog)
    artifacts = [Path(__file__), Path(__file__).with_name('evaluate_scans.py'),
                 a.old_binary, a.new_binary, a.catalog]
    frozen = {str(p.resolve()):sha(p) for p in artifacts}
    write(a.output/'frozen.json', dict(files=frozen,cases=cases,
        scope='16 paired generated raster conditions, four font faces. Known word labels and measured raster boxes, no OCR. Stable word-identity train/validation/audit split. Not independent documents or recovery accuracy.'))
    results = []
    for case in cases:
        directory = a.output/case['id']
        source = json.loads((directory/'input.json').read_text())
        row = dict(case=case['id'], modes={})
        for mode,binary in [('old',a.old_binary),('bounded',a.new_binary)]:
            project = directory/mode
            project.mkdir()
            write(project/'project.json',copy.deepcopy(source))
            cmd = [str(binary.resolve()),'calibrate','--project',str(project),'--page','p','--id','body']
            for font in catalog:
                cmd += ['--font',font['path']]
            proc = subprocess.run(cmd,capture_output=True,text=True,timeout=120)
            (project/'stderr.txt').write_text(proc.stderr)
            (project/'stdout.json').write_text(proc.stdout)
            if proc.returncode:
                row['modes'][mode] = dict(status='error',error=proc.stderr)
            else:
                profile = json.loads(proc.stdout)
                fit = profile['fits'][0]
                row['modes'][mode] = dict(status='ok',font=fit['font'],size=fit['size'],
                    tracking=fit['tracking'],ink_bias=fit['ink_bias'],
                    validation_rmse=fit['validation_rmse'],audit=fit['audit'])
        results.append(row)
        print(json.dumps(row),flush=True)
    assert all(sha(Path(p))==h for p,h in frozen.items())
    write(a.output/'summary.json',dict(cases=results, all_frozen_hashes_verified=True))


if __name__ == '__main__':
    main()
