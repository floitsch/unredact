# Copyright (C) 2026 Toit contributors.
"""Replay successful local solves after validation-only changes, without redoing OCR.

This is a consistency audit, not a new independent benchmark or a substitute for
rerunning adapters when import/calibration/scoring behavior changes.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, action='append', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    binary_hash = digest(binary)
    rows = []
    sources = []
    for root in args.run:
        report = root / 'comparison.json'
        if not report.is_file():
            raise ValueError('Only completed comparisons can be replayed')
        sources.append(dict(path=str(report), sha256=digest(report)))
        for execution in sorted(root.glob('*/execution.json')):
            entry = json.loads(execution.read_text())
            if entry.get('error') or not entry.get('result') or entry['result']['status'] != 'ok':
                continue
            work = execution.parent / 'work'
            expected = work / 'native-result.json'
            vocabulary = work / 'candidates.json'
            project = work / 'project/project.json'
            if not expected.is_file():
                raise ValueError('This replay supports local adapters only')
            count = len(json.loads(vocabulary.read_text())['candidates'])
            output = subprocess.run([str(binary), 'solve', '--project', str(project.parent),
                '--group', 'comparison', '--candidates', str(vocabulary), '--limit', str(count)],
                capture_output=True, text=True, timeout=120)
            identical = output.returncode == 0 and json.loads(output.stdout) == json.loads(expected.read_text())
            rows.append(dict(case=entry['case'], adapter=entry['adapter'], project=str(project),
                project_sha256=digest(project), candidates_sha256=digest(vocabulary),
                expected_sha256=digest(expected), identical=identical,
                error=output.stderr if output.returncode else None))
    if digest(binary) != binary_hash:
        raise ValueError('Replay binary changed during audit')
    result = dict(binary=str(binary), binary_sha256=binary_hash, sources=sources, rows=rows,
        interpretation='Replayed successful solves on unchanged saved OCR/profiles. Failed original executions remain failures; no extra independent trials.')
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    matched = sum(r['identical'] for r in rows)
    print(f'{matched}/{len(rows)} saved results reproduced exactly')
    if matched != len(rows):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
