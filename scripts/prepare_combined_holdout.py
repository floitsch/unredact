# Copyright (C) 2026 Toit contributors.
"""Prepare predeclared new mask locations and a corpus excluding their pages."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import random

from PIL import Image, ImageDraw


def load(path):
    return json.loads(Path(path).read_text())


def write(path, data):
    path.write_text(json.dumps(data, indent=2) + '\n')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def intersects(a, b):
    return (a['x'] < b['x']+b['w'] and b['x'] < a['x']+a['w']
            and a['y'] < b['y']+b['h'] and b['y'] < a['y']+a['h'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    args = parser.parse_args()
    plan = load(args.plan)
    out = args.plan.parent
    source = Path(plan['source_project'])
    assert digest(source) == plan['source_project_sha256']
    project = load(source)
    selected_pages, crops, selections, cases = set(), [], [], []
    for number in plan['pages']:
        page = next(p for p in project['pages'] if Path(p['source']).name == f'page-{number:03}.pgm')
        selected_pages.add(page['id'])
        eligible = []
        for index, word in enumerate(page['words']):
            text, b = word['text'], word['bounds']
            if not (text.isascii() and text.isalpha() and text.islower() and 5 <= len(text) <= 12
                    and word['confidence'] >= 90 and word['line']
                    and b['x'] >= 100 and b['y'] >= 200
                    and b['x']+b['w'] <= page['width']-100
                    and b['y']+b['h'] <= page['height']-200
                    and not any(intersects(b, m) for m in page['proposed_redactions'])):
                continue
            key = hashlib.sha256(f"{plan['selection_seed']}:{number}:{index}:{text}".encode()).hexdigest()
            eligible.append((key, index, word))
        picked, lines = [], set()
        for _, index, word in sorted(eligible):
            line = (word['block'], word['line'])
            if line in lines:
                continue
            picked.append((index, word))
            lines.add(line)
            if len(picked) == plan['words_per_page']:
                break
        assert len(picked) == plan['words_per_page'], f'Insufficient eligible words on page {number}'
        image_path = source.parent / page['image']
        image = Image.open(image_path).convert('L')
        assert hashlib.sha256(image.tobytes()).hexdigest() == page['image_sha256']
        masked = image.copy()
        masks, targets = [], []
        for index, word in picked:
            b = word['bounds']
            crop_box = tuple(int(v) for v in (b['x'], b['y'], b['x']+b['w'], b['y']+b['h']))
            ink = image.crop(crop_box).point(lambda x: 255 if x < 160 else 0).getbbox()
            assert ink is not None
            x, y = crop_box[0]+ink[0], crop_box[1]+ink[1]
            box = dict(x=float(x-2), y=float(y-2), w=float(ink[2]-ink[0]+4), h=float(ink[3]-ink[1]+4))
            assert not any(intersects(box, m) for m in masks)
            masks.append(box)
            identity = f'page-{number}-word-{index}'
            selection = dict(id=identity, page=number, word_index=index, text=word['text'],
                             ocr=word, mask=box, original_pixel_sha256=page['image_sha256'])
            selections.append(selection)
            context_crop = image.crop((max(0,x-200),max(0,y-35),min(image.width,x+int(b['w'])+200),min(image.height,y+int(b['h'])+35)))
            tile = Image.new('L', (1000, 175), 255)
            ImageDraw.Draw(tile).text((8, 4), f"{identity}: {word['text']}", fill=0)
            context_crop.thumbnail((980, 140))
            tile.paste(context_crop, (8,25))
            crops.append(tile)
            targets.append(dict(id=identity, page=f'page-{number}', bounds=box, surface_kind='canonical',
                                evidence=dict(kind='opaque', padding_min=0., padding_max=12., measurement_sigma=1.5),
                                left_context='', right_context='', dependence_key=None))
            ImageDraw.Draw(masked).rectangle((box['x'],box['y'],box['x']+box['w']-1,box['y']+box['h']-1),fill=0)
        name = f'masked-{number:03}.pgm'
        masked.save(out/name)
        entry = dict(id=f'page-{number}', image=name, image_sha256=digest(out/name),
                     dpi=page['dpi'], redaction_regions=masks)
        for target, (_, word) in zip(targets, picked):
            cases.append(dict(id=target['id'], truth_text=word['text'],
                              input=dict(schema_version=1, pages=[entry], targets=[target])))
    sheet = Image.new('L', (1000, 175*len(crops)), 255)
    for i, tile in enumerate(crops):
        sheet.paste(tile, (0,i*175))
    sheet.save(out/'selected-word-review.png')
    truths = sorted({s['text'] for s in selections})
    dictionary = Path('baseline-alex/unredact/data/nouns.txt')
    pool = sorted({w.strip() for w in dictionary.read_text().splitlines()
                   if w.strip().isascii() and w.strip().isalpha() and w.strip().islower()
                   and 5 <= len(w.strip()) <= 12 and w.strip() not in truths})
    words = sorted(truths + random.Random(plan['selection_seed']).sample(pool, plan['closed_vocabulary_count']-len(truths)))
    candidates = [dict(id=hashlib.sha256(w.encode()).hexdigest(), canonical_name=w,
                       surfaces=[dict(text=w,kind='canonical')],source_ids=[],context_score=None) for w in words]
    for case in cases:
        truth = case.pop('truth_text')
        case['truth_id'] = next(c['id'] for c in candidates if c['canonical_name'] == truth)
        case['input']['candidates'] = candidates
    write(out/'closed-candidates.json',dict(candidates=candidates))
    write(out/'suite.json',dict(schema_version=1,track='candidate_ranking_from_images',cases=cases,
        description='Eight predeclared random ordinary-word controlled masks on four previously untested NASA page locations. Supplied300-candidate vocabulary, unknown total padding0..12px; two masks per source page; same-document observations correlated.'))
    reference = copy.deepcopy(project)
    assert not reference['profiles'] and not reference['targets']
    reference['pages'] = [p for p in reference['pages'] if p['id'] not in selected_pages]
    for page in reference['pages']:
        page['image'] = str((source.parent/page['image']).resolve())
    reference_dir = out/'reference-project'
    reference_dir.mkdir(exist_ok=False)
    write(reference_dir/'project.json',reference)
    corpus = load('runs/visible-corpus-extension-001/manifest.json')['corpora']['nasa']
    corpus = [d for d in corpus if Path(d).resolve() != source.parent.resolve()] + [str(reference_dir)]
    write(out/'corpus-projects.json',corpus)
    write(out/'selections.json',dict(plan_sha256=digest(args.plan),script_sha256=digest(__file__),
        source_project_sha256=digest(source),dictionary_sha256=digest(dictionary),selections=selections,
        review='Pending visual inspection of the saved context crops; labels have not entered runtime inputs except the explicit closed vocabulary.'))


if __name__ == '__main__':
    main()
