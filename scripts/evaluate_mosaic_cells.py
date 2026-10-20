# Copyright (C) 2026 Toit contributors.
"""Full development mosaic ranking with cached, identical visible-text typography.

The experimental Rust example consumes only images, masks, typography and the
candidate list. Ground truth and condition labels remain in this controller.
Both engines use the same cached OCR/font fits. This is a scorer comparison,
not fresh end-to-end OCR performance or a comparison with upstream projects.
"""
import argparse
import copy
from collections import Counter, defaultdict
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

import numpy as np
from PIL import Image
from evaluate_common_language import truth_rank
from summarize_multidoc_benchmark import cluster_interval


def load(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def prepare_request(benchmark, case, work):
    project = load(work / 'project/project.json')
    if len(project['targets']) != 1 or len(case['input']['targets']) != 1:
        raise ValueError('Expected one target')
    target = case['input']['targets'][0]
    old_target = project['targets'][0]
    old_page = next(p for p in project['pages'] if p['id'] == old_target['page'])
    page = next(p for p in case['input']['pages'] if p['id'] == target['page'])
    if target['bounds'] != old_target['bounds']:
        raise ValueError('Cached target bounds differ')
    image_path = benchmark / page['image']
    if digest(image_path) != page['image_sha256']:
        raise ValueError('Benchmark image changed')
    old_image = (work / 'project' / old_page['image']).resolve()
    original = np.array(Image.open(old_image).convert('L'))
    image = np.array(Image.open(image_path).convert('L'))
    if original.shape != image.shape:
        raise ValueError('Cached image dimensions differ')
    outside = np.ones(image.shape, dtype=bool)
    for b in page['redaction_regions']:
        if any(int(b[k]) != b[k] for k in ['x', 'y', 'w', 'h']):
            raise ValueError('This pixelation model requires integer masks')
        x, y, w, h = [int(b[k]) for k in ['x', 'y', 'w', 'h']]
        outside[y:y+h, x:x+w] = False
    if not np.array_equal(original[outside], image[outside]):
        raise ValueError('Visible pixels changed; cached typography not a controlled comparison')
    matches = [i for i, b in enumerate(page['redaction_regions']) if b == target['bounds']]
    if len(matches) != 1:
        raise ValueError('Target must match exactly one declared region')
    profile = copy.deepcopy(next(p for p in project['profiles'] if p['id'] == old_target['profile']))
    # Omit free text not needed by the scorer. Numeric fits/corrections remain
    # exactly those learned from the opaque reference's visible text.
    profile['training_texts'] = []
    profile['validation_texts'] = []
    profile['audit_texts'] = []
    request = dict(image=str(image_path.resolve()), regions=page['redaction_regions'], target_index=matches[0],
                   profile=profile, candidates=case['input']['candidates'])
    # A copied legacy project supplies precisely the amended crop-relative
    # origin/baseline. The new scorer estimates positioning from pixels instead.
    old_page['image'] = str(image_path.resolve())
    old_page['image_sha256'] = hashlib.sha256(image.tobytes()).hexdigest()
    old_target['evidence'] = copy.deepcopy(target['evidence'])
    return request, project, old_image


def summarize(rows):
    buckets = defaultdict(list)
    for row in rows:
        for detail, value in [('all', 'all'), ('document', row['document']), ('kind', row['kind'])]:
            buckets[row['condition'], row['availability'], detail, value].append(row)
    summaries, paired = [], []
    for (condition, availability, detail, value), values in sorted(buckets.items()):
        means = {}
        for engine in ['legacy', 'shared_cells']:
            outcomes = [r['engines'][engine] for r in values]
            by_document = defaultdict(list)
            for r, outcome in zip(values, outcomes):
                by_document[r['document']].append(float(outcome.get('truth_rank') == [1, 1]))
            means[engine] = {d: statistics.mean(v) for d, v in sorted(by_document.items())}
            item = dict(condition=condition, availability=availability, breakdown=detail, value=value,
                        engine=engine, cases=len(values), statuses=dict(Counter(o['status'] for o in outcomes)))
            if availability == 'present':
                item.update(unique_correct_first=sum(o.get('truth_rank') == [1, 1] for o in outcomes),
                            truth_renderable=sum(o.get('truth_rank') is not None for o in outcomes),
                            top5_guaranteed=sum(o.get('truth_rank') is not None and o['truth_rank'][1] <= 5 for o in outcomes),
                            document_macro_first=statistics.mean(means[engine].values()),
                            document_95_interval=cluster_interval(list(means[engine].values())))
            else:
                item['absence_accuracy'] = None
                item['absence_note'] = 'No calibrated rejection rule; rankings alone do not detect absent truth.'
            summaries.append(item)
        if availability == 'present' and detail == 'all':
            differences = [means['shared_cells'][d] - means['legacy'][d] for d in sorted(means['legacy'])]
            paired.append(dict(condition=condition, documents=len(differences), cases=len(values),
                               document_macro_difference=statistics.mean(differences),
                               document_95_interval=cluster_interval(differences)))
    return dict(metrics=summaries, paired=paired)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    plan = load(a.plan)
    benchmark, geometry = Path(plan['benchmark']), Path(plan['geometry_root'])
    # This comparison uses every development span, including all failures.
    reference_report = load(geometry / 'eval-8f2a/comparison.json')
    if len(reference_report['executions']) != 384 or any(p.get('error') for p in reference_report['provenance']):
        raise ValueError('Complete reference comparison with valid provenance required')
    reference_suite_path = Path(plan['opaque_benchmark']) / 'development-opaque-present.json'
    if digest(reference_suite_path) != reference_report['suite_sha256']:
        raise ValueError('Reference suite differs')
    reference_suite = load(reference_suite_path)
    by_case = {c['id']: i for i, c in enumerate(reference_suite['cases'])}
    labels = [r for r in load(benchmark / 'controller/labels.json')['labels'] if r['split'] == 'development']
    if len(labels) != 96:
        raise ValueError('Expected all96 development spans')
    label_index = {c['id']: r for r in labels for c in r['cases']}
    binary, legacy = Path(plan['binary']), Path(plan['legacy_binary'])
    source = Path(plan['source'])
    files = load(geometry / 'freeze.json')
    files.update(load(benchmark / 'freeze.json'))
    artifacts = [a.plan, Path(__file__), Path(__file__).with_name('evaluate_common_language.py'),
                 Path(__file__).with_name('summarize_multidoc_benchmark.py'), binary, legacy,
                 geometry / 'eval-8f2a/comparison.json', reference_suite_path,
                 *source.glob('src/*.rs'), *source.glob('native/*'), source / 'examples/rank_mosaic_cells.rs',
                 source / 'Cargo.toml', source / 'Cargo.lock', source / 'build.rs']
    out = a.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    jobs = []
    manifest = load(benchmark / 'manifest.json')
    for condition in ['pixelate-8', 'pixelate-16']:
        for availability in ['present', 'absent']:
            suite_path = benchmark / f'development-{condition}-{availability}.json'
            if digest(suite_path) != manifest['suites'][suite_path.stem]['sha256']:
                raise ValueError('Suite hash mismatch')
            artifacts.append(suite_path)
            suite = load(suite_path)
            if len(suite['cases']) != 96:
                raise ValueError('Full96-case suite required')
            for case in suite['cases']:
                label = label_index[case['id']]
                ref_id = next(c['id'] for c in label['cases'] if c['condition'] == 'opaque' and c['availability'] == 'present')
                index = by_case[ref_id]
                entry = next(e for e in reference_report['executions'] if e['case'] == ref_id and e['adapter'] == 'local-profile-cost')
                if entry['error'] or entry['result']['status'] != 'ok':
                    raise ValueError('Reference cache failed; cannot silently omit planned case')
                work = geometry / 'eval-8f2a' / f'case-{index:03}-repeat-00-tool-00/work'
                request, project, old_image = prepare_request(benchmark, case, work)
                directory = out / case['id']
                directory.mkdir()
                (directory / 'project').mkdir()
                write(directory / 'request.json', request)
                write(directory / 'project/project.json', project)
                write(directory / 'candidates.json', dict(candidates=case['input']['candidates']))
                artifacts += [work / 'project/project.json', old_image, Path(request['image']),
                              directory / 'request.json', directory / 'project/project.json', directory / 'candidates.json']
                artifacts += [Path(f['font']) for f in request['profile']['fits']]
                jobs.append(dict(case=case, label=label, directory=directory, condition=condition, availability=availability))
    files.update({str(p.resolve()): digest(p) for p in artifacts})
    def verify():
        for path, sha in files.items():
            if digest(path) != sha:
                raise ValueError(f'Frozen file changed: {path}')
    verify()
    write(out / 'freeze.json', dict(files=files, plan=plan, cases=len(jobs)))
    rows = []
    for job in jobs:
        case, label, directory = job['case'], job['label'], job['directory']
        row = dict(case=case['id'], document=label['document'], kind=label['kind'],
                   condition=job['condition'], availability=job['availability'], engines={})
        for engine, command in [('legacy', [legacy, 'solve', '--project', directory / 'project', '--group', 'comparison',
                                            '--candidates', directory / 'candidates.json', '--limit', '300']),
                                ('shared_cells', [binary, directory / 'request.json'])]:
            start = time.monotonic()
            try:
                proc = subprocess.run(list(map(str, command)), capture_output=True, text=True, timeout=plan['timeout_seconds'],
                                      env={**os.environ, 'OMP_THREAD_LIMIT': '1'})
                (directory / f'{engine}.stderr').write_text(proc.stderr)
                (directory / f'{engine}.json').write_text(proc.stdout)
                if proc.returncode:
                    outcome = dict(status='error', reason=proc.stderr[-2000:])
                else:
                    result = json.loads(proc.stdout)
                    scores = ({c['id']: -c['evidence_score'] for c in result['candidates']} if engine == 'legacy' else
                              {c['id']: -c['cost'] for c in result['scores'] if c['cost'] is not None})
                    outcome = dict(status='ok', renderable_candidates=len(scores),
                                   truth_rank=truth_rank(scores, case['truth_id']) if job['availability'] == 'present' else None)
            except subprocess.TimeoutExpired:
                outcome = dict(status='timeout', reason='Fixed execution timeout exceeded')
            outcome['elapsed_seconds'] = time.monotonic() - start
            row['engines'][engine] = outcome
        rows.append(row)
        write(directory / 'evaluation.json', row)
        write(out / 'progress.json', dict(completed=len(rows), planned=len(jobs)))
        print(json.dumps(dict(completed=len(rows), case=case['id'])), flush=True)
    verify()
    write(out / 'summary.json', dict(rows=rows, **summarize(rows), all_frozen_hashes_verified=True,
                                    interpretation=plan['interpretation']))


if __name__ == '__main__':
    main()
