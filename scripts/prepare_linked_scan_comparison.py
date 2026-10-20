# Copyright (C) 2026 Toit contributors.
"""Compare each reviewed scan occurrence with its supplied two-occurrence link.

Only previously prepared masked images enter requests. Truth labels select the
evaluation cases; the adapter inputs contain geometry, vocabulary and links only.
"""
import argparse
from collections import defaultdict
import copy
import hashlib
import json
from pathlib import Path
import shutil


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    source = json.loads((a.source / 'suite.json').read_text())
    buckets = defaultdict(list)
    for case in source['cases']:
        if any(c['id'] == case['truth_id'] for c in case['input']['candidates']):
            buckets[case['truth_id']].append(case)
    a.output.mkdir(parents=True, exist_ok=False)
    cases, pairs, copied = [], [], {}
    for index, (truth, originals) in enumerate(sorted(buckets.items())):
        if len(originals) != 2 or any(len(c['input']['targets']) != 1 or len(c['input']['pages']) != 1 for c in originals):
            raise ValueError('Each identity must have exactly two single-occurrence cases')
        vocabulary = originals[0]['input']['candidates']
        if originals[1]['input']['candidates'] != vocabulary:
            raise ValueError('Paired vocabularies differ')
        pages, targets = [], []
        for case in originals:
            page = copy.deepcopy(case['input']['pages'][0])
            image = (a.source / page['image']).resolve()
            if digest(image) != page['image_sha256']:
                raise ValueError('Source masked pixels changed')
            name = image.name
            if name in copied and copied[name] != page['image_sha256']:
                raise ValueError('Conflicting masked image filenames')
            if name not in copied:
                shutil.copy2(image, a.output / name)
                copied[name] = page['image_sha256']
            page['image'] = name
            target = copy.deepcopy(case['input']['targets'][0])
            target['evidence'] = dict(kind='opaque', padding_min=0., padding_max=12., measurement_sigma=1.5)
            pages.append(page)
            targets.append(target)
        if len({p['id'] for p in pages}) != 2 or len({p['image_sha256'] for p in pages}) != 2:
            raise ValueError('Require two distinct page images, not duplicate views')
        pair = dict(id=f'pair-{index:03}', truth_id=truth, targets=[t['id'] for t in targets], cases=[])
        for condition, selected in [('first', [0]), ('second', [1]), ('joint', [0, 1])]:
            for present in [True, False]:
                identity = f"{condition}-{'present' if present else 'absent'}-pair-{index:03}"
                data = dict(schema_version=1, pages=[pages[i] for i in selected],
                    targets=[targets[i] for i in selected],
                    candidates=[c for c in vocabulary if present or c['id'] != truth])
                cases.append(dict(id=identity, truth_id=truth, input=data))
                pair['cases'].append(identity)
        pairs.append(pair)
    suite = dict(schema_version=1, track='candidate_ranking_from_images', cases=cases,
        description='Fresh page locations in a previously used real scan document. Each of six supplied same-text pairs is tested first-only, second-only and jointly, with truth-present/absent vocabulary. Identical masked pixels; unknown total padding 0–12 pixels. Original text was visible before controlled masking. Links are supplied, not inferred.')
    (a.output / 'suite.json').write_text(json.dumps(suite, indent=2) + '\n')
    (a.output / 'provenance.json').write_text(json.dumps(dict(source_suite_sha256=digest(a.source / 'suite.json'),
        script_sha256=digest(Path(__file__)), pairs=pairs, copied_images=copied,
        interpretation='Pairs and single views reuse observations; six identities, twelve occurrences, not 36 independent examples. Same-document correlations and unmasked repeats remain. Actual mask padding is withheld from adapter inputs.'), indent=2) + '\n')
    print(a.output / 'suite.json')


if __name__ == '__main__':
    main()
