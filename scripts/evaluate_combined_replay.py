# Copyright (C) 2026 Toit contributors.
"""Check fixed combined ranking against archived inference; no new model calls."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from evaluate_span_ranking import rank


def load(path):
    return json.loads(Path(path).read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    cases = load(a.manifest)['cases']
    binary = Path('target/release/unredact').resolve()
    python = Path('runs/baseline-venv/bin/python').absolute()
    provider = Path('scripts/evaluate_bounded_controls.py').resolve()
    paths = [a.manifest,Path(__file__),binary,provider,Path('src/contextual.rs')]
    for c in cases:
        paths += [Path(c['source_report']),Path(c['candidates']),Path(c['project'])/'project.json']
    frozen = {str(path):digest(path) for path in paths}
    write(a.output/'frozen.json',dict(files=frozen,weight=1.,scope='Reused development cases and exact cached T5 scores. No tuning or independent accuracy claim.'))
    rows = []
    for i,case in enumerate(cases):
        directory = a.output/f'case-{i:03}'
        directory.mkdir()
        source = load(case['source_report'])
        traces = source['provider'].get('batches',[source['provider']])
        cache = dict(request=dict(schema_version=1,task='score_context_candidates',contexts=source['contexts'],
                                 items=[item for t in traces for item in t['request']['items']]),
                     scores=[s for t in traces for s in t['response']['scores']])
        cached = directory/'cached-scores.json'
        write(cached,cache)
        row = dict(case=case['id'],track=case['track'],cohort=case['cohort'])
        for weight in [0,1]:
            path = directory/f'weight-{weight}.json'
            cmd = [str(binary),'context-rank','--project',case['project'],'--group','comparison',
                '--candidates',case['candidates'],'--paragraph-context','--sequence-score','--physical-weight',str(weight),
                '--provider',str(python),'--provider-arg',str(provider),'--provider-arg=--replay',
                '--provider-arg',str(cached.resolve()),'--output',str(path)]
            proc = subprocess.run(cmd,capture_output=True,text=True,timeout=90)
            (directory/f'weight-{weight}.stderr').write_text(proc.stderr)
            assert proc.returncode == 0,proc.stderr
            report = load(path)
            for key in ['physical','contexts','normalization']:
                assert report[key] == source[key],(i,key)
            if weight == 0:
                for key in ['ranked','ranked_groups']:
                    assert report[key] == source[key],(i,key)
                assert 'physical_weight' not in report
            else:
                old = {r['id']:r for r in source['ranked']}
                physical = {r['id']:r for r in source['physical']['candidates']}
                for r in report['ranked']:
                    assert r['observations'] == old[r['id']]['observations']
                    assert r['mean_sequence_log_score'] == old[r['id']]['mean_sequence_log_score']
                    assert r['mean_physical_cost'] == physical[r['id']]['evidence_score']/len(r['observations'])
                    assert r['combined_score'] == r['mean_sequence_log_score'] - r['mean_physical_cost']
            row[f'weight_{weight}_rank'] = rank(report['ranked_groups'],case['truth_ids'])
        rows.append(row)
        write(directory/'evaluation.json',row)
        print(json.dumps(row),flush=True)
    assert all(digest(path) == sha for path,sha in frozen.items())
    totals = {}
    for row in rows:
        key = row['track']+'::'+row['cohort']
        counts = totals.setdefault(key,dict(cases=0,language_first=0,combined_first=0))
        counts['cases'] += 1
        counts['language_first'] += row['weight_0_rank'] == [1,1]
        counts['combined_first'] += row['weight_1_rank'] == [1,1]
    write(a.output/'summary.json',dict(totals=totals,rows=rows,all_frozen_hashes_verified=True,
        all_default_scores_contexts_physics_and_ranks_identical=True))


if __name__ == '__main__':
    main()
