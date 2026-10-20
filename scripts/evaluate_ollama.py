# Copyright (C) 2026 Toit contributors.
"""Small explicit-label local-LLM smoke suite; not a recovery accuracy benchmark."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time

# Freeze these cases before running. The envelope case was used in prompt
# development and is excluded from the fresh-case recall denominator.
CASES = [
    ('development-envelope', 'envelope', [('She sealed the letter in an ', ' and put a stamp on it.')]),
    ('temperature', 'thermometer', [('The nurse measured his temperature with a ', '.')]),
    ('vote', 'unanimous', [('Every member voted in favor, so the decision was ', '.')]),
    ('weather', 'postponed', [('Because of the storm, the match was ', ' until the following day.')]),
    ('comparison', 'compared', [('The analyst ', ' the two reports and noted their differences.')]),
    ('linked-table', 'table', [('The results are summarized in the ', ' below.'),
                              ('Each row of the ', ' represents one experiment.')]),
]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--endpoint', default='http://127.0.0.1:11434')
    p.add_argument('--model', default='llama3.2:3b')
    p.add_argument('--count', type=int, default=12)
    p.add_argument('--binary', type=Path, default=Path('target/release/unredact'))
    p.add_argument('--geometry-project', type=Path)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    provider = Path(__file__).with_name('ollama_provider.py').resolve()
    base = [sys.executable, str(provider), '--endpoint', a.endpoint, '--model', a.model,
            '--count', str(a.count), '--trace', str((a.output/'trace.jsonl').resolve())]
    results = []
    for name, truth, contexts in CASES:
        request = {'schema_version': 1, 'task': 'suggest_candidates', 'group': name,
            'targets': [{'id': str(i), 'surface_kind': 'canonical', 'left_context': left,
                         'right_context': right} for i, (left, right) in enumerate(contexts)],
            'existing_candidates': []}
        (a.output/f'{name}-request.json').write_text(json.dumps(request, indent=2))
        start = time.monotonic()
        proc = subprocess.run(base, input=json.dumps(request), capture_output=True, text=True, timeout=300)
        (a.output/f'{name}-stdout.json').write_text(proc.stdout)
        (a.output/f'{name}-stderr.txt').write_text(proc.stderr)
        texts = [] if proc.returncode else [c['surfaces'][0]['text'] for c in json.loads(proc.stdout)['candidates']]
        results.append(dict(case=name, truth=truth, development=name.startswith('development'),
            status='ok' if proc.returncode == 0 else 'error', proposals=texts,
            exact_truth_proposed=truth in texts, seconds=time.monotonic()-start))
        print(name, results[-1]['status'], 'truth proposed:', truth in texts, flush=True)
    # Exercise incremental transport with a known prefix and explicit spaces.
    request = {'schema_version': 2, 'task': 'extend_prefixes', 'surface_kind': 'canonical',
        'targets': [{'left_context': 'He sent the parcel by ', 'right_context': ' to ensure next-day delivery.'}],
        'prefixes': [{'id': 'prefix-express', 'text': 'express ', 'observations': []}],
        'limits': {'max_extensions': 8, 'max_text_bytes': 128}, 'completed_feedback': []}
    proc = subprocess.run(base, input=json.dumps(request), capture_output=True, text=True, timeout=300)
    (a.output/'incremental-request.json').write_text(json.dumps(request, indent=2))
    (a.output/'incremental-stdout.json').write_text(proc.stdout)
    (a.output/'incremental-stderr.txt').write_text(proc.stderr)
    incremental = dict(status='ok' if proc.returncode == 0 else 'error')
    if not proc.returncode:
        incremental['response'] = json.loads(proc.stdout)
    geometry = None
    if a.geometry_project:
        project_path = a.output/'geometry-project'
        shutil.copytree(a.geometry_project, project_path)
        project_file = project_path/'project.json'
        project = json.loads(project_file.read_text())
        # Controlled visible word masked earlier in document suite. Original
        # redactions remain untouched. Context spelling corrected from OCR.
        assert project['targets'][0]['id'] == 'visible-word-0'
        project['targets'][0]['left_context'] = 'is '
        project['targets'][0]['right_context'] = ' to live in NY but was away for the holiday weekend.'
        project_file.write_text(json.dumps(project, indent=2))
        args = [str(a.binary.resolve()), 'solve', '--project', str(project_path),
                '--group', project['targets'][0]['group'], '--provider', sys.executable,
                '--timeout-seconds', '300', '--limit', '100']
        for arg in base[1:]:
            args.append('--provider-arg='+arg)
        proc = subprocess.run(args, capture_output=True, text=True, timeout=320)
        (a.output/'geometry-result.json').write_text(proc.stdout)
        (a.output/'geometry-stderr.txt').write_text(proc.stderr)
        geometry = dict(status='ok' if proc.returncode == 0 else 'error', truth='confirmed')
        if not proc.returncode:
            report = json.loads(proc.stdout)
            geometry['candidates'] = [{k: c[k] for k in ('name', 'compatible', 'evidence_score')}
                                      for c in report['candidates']]
            geometry['truth_proposed'] = any(c['name']=='confirmed' for c in report['candidates'])
        print('geometry:', geometry['status'], flush=True)
    fresh = [r for r in results if not r['development']]
    summary = dict(model=a.model, count=a.count, cases=results, incremental=incremental,
        geometry=geometry, fresh_exact_recall=[sum(r['exact_truth_proposed'] for r in fresh),len(fresh)],
        script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        provider_sha256=hashlib.sha256(provider.read_bytes()).hexdigest(),
        limitation='Tiny hand-written smoke cases; envelope used in prompt development. No redaction recovery rate or external baseline superiority established.')
    (a.output/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')


if __name__ == '__main__':
    main()
