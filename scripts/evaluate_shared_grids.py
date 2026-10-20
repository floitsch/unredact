# Copyright (C) 2026 Toit contributors.
"""Compare shared-region grid detection with the completed single-region audit."""
import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import subprocess


def load(path):
    return json.loads(Path(path).read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--audit', type=Path, required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--source', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    plan = load(a.audit / 'plan.json')
    baseline = load(a.audit / 'summary.json')
    if len(baseline['rows']) != 672 or not baseline['frozen_verified']:
        raise ValueError('Complete single-region audit required')
    frozen = dict(plan['files'])
    for path in [Path(__file__), a.audit / 'plan.json', a.audit / 'summary.json', a.binary,
                 *a.source.glob('src/*.rs'), a.source / 'examples/shared_grid.rs',
                 a.source / 'Cargo.toml', a.source / 'Cargo.lock', a.source / 'build.rs']:
        frozen[str(path.resolve())] = digest(path)
    for path, sha in frozen.items():
        if digest(path) != sha:
            raise ValueError(f'Frozen source changed: {path}')
    a.output.mkdir(parents=True, exist_ok=False)
    (a.output / 'freeze.json').write_text(json.dumps(frozen, indent=2) + '\n')
    grouped = defaultdict(list)
    for task in plan['tasks']:
        grouped[task['condition'], task['path']].append(task)
    by_id = {r['id']: r for r in baseline['rows']}
    rows, page_rows = [], []
    for (condition, page), tasks in sorted(grouped.items()):
        if len(tasks) != 8:
            raise ValueError('Expected all eight disjoint regions on each page')
        page_sha = digest(page)
        image = a.output / f'{page_sha}.pgm'
        if not image.exists():
            os.link(page, image)
        regions = [dict(x=t['box'][0], y=t['box'][1], w=t['box'][2] - t['box'][0], h=t['box'][3] - t['box'][1]) for t in tasks]
        identity = hashlib.sha256(json.dumps(dict(image=page_sha, regions=regions), sort_keys=True).encode()).hexdigest()
        region_path = a.output / f'{identity}-regions.json'
        region_path.write_text(json.dumps(regions) + '\n')
        run = subprocess.run([str(a.binary.resolve()), str(image.resolve()), str(region_path.resolve())],
                             capture_output=True, text=True, timeout=30)
        (a.output / f'{identity}.stderr').write_text(run.stderr)
        if run.returncode:
            raise RuntimeError(run.stderr)
        report = json.loads(run.stdout)
        (a.output / f'{identity}.json').write_text(run.stdout)
        page_rows.append(dict(condition=condition, document=tasks[0]['document'], image_sha256=page_sha,
                              regions=8, hypotheses=report['hypotheses']))
        for index, task in enumerate(tasks):
            original = by_id[task['id']]
            projected = []
            for h in report['hypotheses']:
                if index not in h['supporting_regions']:
                    continue
                g = h['grid']
                projected.append(dict(block=g['block'], phase_x=(g['phase_x'] + task['box'][0]) % g['block'],
                                      phase_y=(g['phase_y'] + task['box'][1]) % g['block']))
            expected = original['expected']
            rows.append(dict(id=task['id'], condition=condition, document=task['document'], kind=task['kind'],
                             grids=projected, detected=bool(projected), expected=expected,
                             exact_first=expected is not None and bool(projected) and projected[0] == expected,
                             exact_retained=expected is not None and expected in projected,
                             baseline_detected=original['detected'], baseline_exact_first=original['exact_first'],
                             baseline_exact_retained=original['exact_retained']))
    buckets = defaultdict(list)
    for row in rows:
        for kind in ['all', row['kind']]:
            buckets[row['condition'], kind].append(row)
    summary = [dict(condition=c, kind=k, cases=len(v),
                    **{metric: sum(r[metric] for r in v) for metric in ['detected', 'exact_first', 'exact_retained',
                       'baseline_detected', 'baseline_exact_first', 'baseline_exact_retained']})
               for (c, k), v in sorted(buckets.items())]
    for path, sha in frozen.items():
        if digest(path) != sha:
            raise ValueError(f'Frozen source changed: {path}')
    (a.output / 'summary.json').write_text(json.dumps(dict(rows=rows, pages=page_rows, summary=summary,
        frozen_verified=True, interpretation='Development-only grid detection. Shared algorithm gets all eight declared regions and their page coordinates; baseline gets each crop separately. This is the intended extra evidence, not an equal-information algorithm comparison. Shared phases projected only into locally supporting regions. Conditions share spans/documents. No text identity/recovery or held-out accuracy is measured.'), indent=2) + '\n')
    for r in summary:
        if r['kind'] == 'all':
            print(json.dumps(r))


if __name__ == '__main__':
    main()
