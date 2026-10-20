# Copyright (C) 2026 Toit contributors.
"""Create controlled masks over reviewed visible words on new real scan pages.

Source page/crop/text selection is supplied before scoring. OCR locates reviewed
labels but cannot choose or replace them. Labels and originals stay out of adapter
requests. Every selected region is masked in all cases; unselected occurrences
of the same word may remain visible and are recorded explicitly.
"""
import argparse
import hashlib
import json
from pathlib import Path
import random

import numpy as np
from PIL import Image, ImageDraw
import pytesseract


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def matching_spans(ocr, locator):
    """Locate exact reviewed words or same-line phrases, without crossing gutters."""
    wanted = locator.split()
    if not wanted:
        return []
    words = [i for i, text in enumerate(ocr['text']) if text.strip()]
    spans = []
    for start in range(len(words) - len(wanted) + 1):
        span = words[start:start + len(wanted)]
        if [ocr['text'][i].strip() for i in span] != wanted:
            continue
        if any(len({ocr[key][i] for i in span}) != 1 for key in ['block_num', 'par_num', 'line_num']):
            continue
        valid = True
        for a, b in zip(span, span[1:]):
            gap = int(ocr['left'][b]) - int(ocr['left'][a]) - int(ocr['width'][a])
            height = max(int(ocr['height'][a]), int(ocr['height'][b]))
            overlap = min(int(ocr['top'][a])+int(ocr['height'][a]), int(ocr['top'][b])+int(ocr['height'][b])) - max(int(ocr['top'][a]), int(ocr['top'][b]))
            if not 0 <= gap <= 3*height or overlap < .25*min(int(ocr['height'][a]),int(ocr['height'][b])):
                valid = False
        if valid:
            spans.append(span)
    return spans


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--review', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--vocabulary', type=Path, help='Optional newline-separated distractor pool; default is baseline noun dictionary')
    p.add_argument('--candidate-count', type=int, default=400)
    p.add_argument('--seed', type=int, default=9953)
    p.add_argument('--phrase-vocabulary', action='store_true', help='Allow lowercase alphabetic distractors of 1–3 words, up to 80 characters')
    a = p.parse_args()
    root = Path(__file__).resolve().parents[1]
    review = json.loads(a.review.read_text())
    out = a.output.resolve(); out.mkdir(parents=True, exist_ok=False)
    truths = sorted({t if isinstance(t,str) else t['text']
                     for page in review['pages'] for t in page['words']})
    dictionary = a.vocabulary or root/'baseline-alex/unredact/data/nouns.txt'
    minimum_length = 4 if a.vocabulary else 5
    def usable(text):
        if a.phrase_vocabulary:
            return text.islower() and 4 <= len(text) <= 80 and 1 <= len(text.split()) <= 3 and all(w.isalpha() for w in text.split())
        return text.islower() and text.isalpha() and minimum_length <= len(text) <= 12
    population = list(dict.fromkeys(t.strip() for t in dictionary.read_text().splitlines()
        if usable(t.strip()) and t.strip() not in truths))[:10000]
    if not len(truths) <= a.candidate_count <= len(population) + len(truths):
        raise ValueError('Candidate count must include all truths and fit the distractor pool')
    vocabulary = sorted([*random.Random(a.seed).sample(population,a.candidate_count-len(truths)),*truths])
    candidates = [dict(id=hashlib.sha256(t.encode()).hexdigest()[:16], canonical_name=t,
        surfaces=[dict(text=t,kind='canonical')],source_ids=[],context_score=None) for t in vocabulary]
    cases, sources = [], []
    for index, spec in enumerate(review['pages']):
        source = (a.review.parent/spec['image']).resolve()
        if digest(source) != spec['sha256']: raise ValueError('Reviewed image changed')
        original = Image.open(source).convert('L').crop(spec['crop'])
        original.save(out/f'original-{index}.pgm')
        ocr = pytesseract.image_to_data(original, config='--psm 6', output_type=pytesseract.Output.DICT)
        targets = []
        for wi, selection in enumerate(spec['words']):
            truth = selection if isinstance(selection,str) else selection['text']
            locator = truth if isinstance(selection,str) else selection.get('ocr_text', truth)
            spans = matching_spans(ocr, locator)
            indices = [span[0] for span in spans]
            if isinstance(selection,str) and len(indices) != 1:
                raise ValueError(f'Expected exactly one reviewed word {truth} on page {spec["page"]}; found {len(indices)}')
            occurrence = 0 if isinstance(selection,str) else selection['occurrence']
            if not isinstance(occurrence,int) or not 0 <= occurrence < len(indices):
                raise ValueError('Reviewed occurrence is missing')
            span = spans[occurrence]
            x, y = min(int(ocr['left'][i]) for i in span), min(int(ocr['top'][i]) for i in span)
            w = max(int(ocr['left'][i])+int(ocr['width'][i]) for i in span)-x
            h = max(int(ocr['top'][i])+int(ocr['height'][i]) for i in span)-y
            trim_right = 0 if isinstance(selection,str) else selection.get('trim_right', 0)
            trim_bottom = 0 if isinstance(selection,str) else selection.get('trim_bottom', 0)
            trim_top = 0 if isinstance(selection,str) else selection.get('trim_top', 0)
            if type(trim_right) is not int or not 0 <= trim_right < w:
                raise ValueError('Reviewed right trim must be a nonnegative pixel count smaller than OCR width')
            if type(trim_top) is not int or not 0 <= trim_top < h:
                raise ValueError('Reviewed top trim must be a nonnegative pixel count smaller than OCR height')
            if type(trim_bottom) is not int or not 0 <= trim_bottom < h - trim_top:
                raise ValueError('Reviewed bottom trim must be a nonnegative pixel count smaller than OCR height')
            # Explicit, visually reviewed punctuation exclusion; never inferred
            # from candidate rankings. Keep the original locator in provenance.
            w -= trim_right
            y += trim_top
            h -= trim_top + trim_bottom
            yy,xx = np.where(np.asarray(original.crop((x,y,x+w,y+h))) < 160)
            if not len(xx): raise ValueError('Reviewed word contains no dark pixels')
            ink = dict(x=float(x+xx.min()), y=float(y+yy.min()),
                       w=float(xx.max()-xx.min()+1),h=float(yy.max()-yy.min()+1))
            box = dict(x=ink['x']-2,y=ink['y']-2,w=ink['w']+4,h=ink['h']+4)
            targets.append(dict(id=f'page-{spec["page"]}-word-{wi}',bounds=box,ink=ink,truth=truth,
                ocr_locator=locator,
                ocr_word_indices=span,
                raw_ocr_words=[ocr['text'][i] for i in span],
                reviewed_trim_right=trim_right,
                reviewed_trim_bottom=trim_bottom,
                reviewed_trim_top=trim_top,
                selected_occurrence=occurrence, same_text_occurrences=len(indices),
                unselected_same_text_occurrences=len(indices)-1))
        masked = original.copy(); draw=ImageDraw.Draw(masked)
        for t in targets:
            b=t['bounds']; draw.rectangle((b['x'],b['y'],b['x']+b['w']-1,b['y']+b['h']-1), fill=0)
        path=out/f'masked-{index}.pgm';masked.save(path)
        dpi = float(spec.get('dpi', 400.))
        if not 36 <= dpi <= 1200: raise ValueError('Invalid reviewed source DPI')
        page=dict(id=f'page-{spec["page"]}',image=path.name,image_sha256=digest(path),dpi=dpi,
                  redaction_regions=[t['bounds'] for t in targets])
        for t in targets:
            identity=next(c['id'] for c in candidates if c['canonical_name']==t['truth'])
            target=dict(id=t['id'],page=page['id'],bounds=t['bounds'],surface_kind='canonical',
                evidence=dict(kind='width',width=t['ink']['w'],measurement_sigma=1.5),
                left_context='',right_context='',dependence_key=None)
            data=dict(schema_version=1,pages=[page],targets=[target],candidates=candidates)
            cases.append(dict(id='present-'+t['id'],truth_id=identity,input=data))
            cases.append(dict(id='absent-'+t['id'],truth_id=identity,
                input={**data,'candidates':[c for c in candidates if c['id']!=identity]}))
        sources.append(dict(source=str(source),source_sha256=digest(source),crop=spec['crop'],
            page=spec['page'],dpi=dpi,targets=targets,original_sha256=digest(out/f'original-{index}.pgm')))
    suite=dict(schema_version=1,track='candidate_ranking_from_images',cases=cases,
        description=review.get('description','New Senate scan body crops') + '; manually reviewed visible ordinary words; all target words masked; '
        f'{a.candidate_count} candidates; known padding-corrected ink widths; original redactions, if any, untouched/unlabeled.')
    (out/'suite.json').write_text(json.dumps(suite,indent=2)+'\n')
    (out/'provenance.json').write_text(json.dumps(dict(review_sha256=digest(a.review),
        script_sha256=digest(Path(__file__)),dictionary_sha256=digest(dictionary),sources=sources,
        dictionary=str(dictionary.resolve()),candidate_count=a.candidate_count,seed=a.seed,
        phrase_vocabulary=a.phrase_vocabulary,
        limitations='Conditional on reviewed readable words/body crops; within-document observations correlated; '
        'not an accuracy estimate for unknown redactions.'),indent=2)+'\n')
    print(out/'suite.json')


if __name__ == '__main__': main()
