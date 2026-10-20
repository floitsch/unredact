# Copyright (C) 2026 Toit contributors.
"""Compare T5 sequence ranking with saved BERT reports on identical candidates.

Manifest labels grade outputs only. Runtime/inputs are hashed, physical reports
and visible context must remain identical, and source projects are read-only.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def rank(groups, truth_ids):
    offset = 0
    for group in groups:
        if set(group).intersection(truth_ids):
            return [offset + 1, offset + len(group)]
        offset += len(group)
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=Path('target/release/unredact'))
    parser.add_argument('--python', type=Path, default=Path('runs/neural-venv/bin/python'))
    parser.add_argument('--model-dir', type=Path, default=Path('runs/span-model-001/t5-base'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    args.output.mkdir(parents=True, exist_ok=False)
    binary_hash = digest(args.binary)
    rows = []
    for index, case in enumerate(manifest['cases']):
        path = args.output / f'case-{index:03}'
        path.mkdir()
        control = json.loads(Path(case['control']).read_text())
        watched = [case['control'], case['candidates'], str(Path(case['project']) / 'project.json')]
        hashes = {name: digest(name) for name in watched}
        command = [str(args.binary.resolve()), 'context-rank', '--project', case['project'],
            '--group', 'comparison', '--candidates', case['candidates'], '--paragraph-context',
            '--sequence-score', '--provider', str(args.python), '--provider-arg',
            str(Path(__file__).with_name('span_context_provider.py').resolve()),
            '--provider-arg=--model-dir', '--provider-arg', str(args.model_dir),
            '--output', str(path / 'ranking.json')]
        start = time.monotonic()
        result = subprocess.run(command, capture_output=True, text=True, timeout=330)
        (path / 'stderr.txt').write_text(result.stderr)
        row = dict(case=case['id'], track=case['track'], cohort=case['cohort'],
            command=command, seconds=time.monotonic() - start, input_hashes=hashes,
            status='error' if result.returncode else 'ok')
        if not result.returncode:
            scored = json.loads((path / 'ranking.json').read_text())
            if scored['physical'] != control['physical'] or scored['contexts'] != control['contexts']:
                raise ValueError('Physical evidence or visible context changed')
            if scored['normalization'] != 'sequence_sum_mean_over_occurrences':
                raise ValueError('Sequence scoring was not selected')
            # Labels enter here, only after model scoring has completed.
            truth_ids = case['truth_ids']
            row.update(truth_rank=rank(scored['ranked_groups'], truth_ids),
                       control_truth_rank=rank(control['ranked_groups'], truth_ids),
                       truth_available=bool(truth_ids),
                       compatible_candidates=scored['physical']['compatible_candidates'])
        if digest(args.binary) != binary_hash or any(digest(name) != sha for name, sha in hashes.items()):
            raise ValueError('Runtime or input artifact changed')
        rows.append(row)
        (path / 'evaluation.json').write_text(json.dumps(row, indent=2) + '\n')
        print(row['track'], row['cohort'], row['case'], row['status'], row.get('control_truth_rank'), '->', row.get('truth_rank'), flush=True)
        if result.returncode:
            raise RuntimeError('Ranking failed; inspect saved artifacts before continuing')
    summary = {}
    for row in rows:
        key = row['track'] + '::' + row['cohort']
        totals = summary.setdefault(key, dict(cases=0, successful=0, truth_available=0,
                                              bert_unique_first=0, t5_unique_first=0))
        totals['cases'] += 1
        totals['successful'] += row['status'] == 'ok'
        totals['truth_available'] += row.get('truth_available', False)
        totals['bert_unique_first'] += row.get('control_truth_rank') == [1, 1]
        totals['t5_unique_first'] += row.get('truth_rank') == [1, 1]
    (args.output / 'summary.json').write_text(json.dumps(dict(summary=summary, rows=rows,
        manifest_sha256=digest(args.manifest), binary_sha256=binary_hash,
        script_sha256=digest(Path(__file__)),
        interpretation='Reused development cases. Candidate sets, contexts and physical evidence are identical within each paired comparison. Tracks reuse locations and are not independent trials. No matched upstream AI or new holdout claim.'), indent=2) + '\n')
    print(json.dumps(summary), flush=True)


if __name__ == '__main__':
    main()
