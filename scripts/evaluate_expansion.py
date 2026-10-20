# Copyright (C) 2026 Toit contributors.
"""Replay controlled truth-absent cases with an open proposal hook.

Labels grade outputs only. Existing images, calibration and vocabulary are reused;
this is development evidence, not another fresh or matched closed-vocabulary test.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--comparison', type=Path, required=True)
    p.add_argument('--suite', type=Path, required=True)
    p.add_argument('--adapter', default='width-fringe-default-filtered')
    p.add_argument('--binary', type=Path, default=Path('target/release/unredact'))
    p.add_argument('--provider', type=Path, required=True)
    p.add_argument('--provider-arg', action='append', default=[])
    p.add_argument('--context-python', type=Path, required=True)
    p.add_argument('--context-model', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--width-guidance', action='store_true')
    p.add_argument('--case-prefix', default='', help='Evaluate one predeclared condition from a mixed suite')
    a = p.parse_args()
    report = json.loads((a.comparison/'comparison.json').read_text())
    if report['suite_sha256'] != digest(a.suite):
        raise ValueError('Suite mismatch')
    suite = json.loads(a.suite.read_text())
    cases = {c['id']: c for c in suite['cases']}
    labels = {c['id']: c['canonical_name'] for case in suite['cases'] for c in case['input']['candidates']}
    a.output.mkdir(parents=True, exist_ok=False)
    binary_hash = digest(a.binary)
    selected, seen = [], set()
    for path in sorted(a.comparison.glob('*/execution.json')):
        e = json.loads(path.read_text())
        if e['adapter'] != a.adapter or e['error'] or e['metrics']['truth_in_vocabulary'] or not e['case'].startswith(a.case_prefix):
            continue
        if e['case'] in seen:
            raise ValueError('Repetitions are not independent cases')
        seen.add(e['case'])
        selected.append((path, e))
    if not selected:
        raise ValueError('No successful absent-vocabulary executions')
    rows = []
    for index, (path, execution) in enumerate(selected):
        out = a.output/f'case-{index:03d}'
        out.mkdir()
        project = path.parent/'work/project'
        candidate_path = path.parent/'work/candidates.json'
        case = cases[execution['case']]
        candidates = json.loads(candidate_path.read_text())['candidates']
        if candidates != case['input']['candidates'] or any(c['id'] == case['truth_id'] for c in candidates):
            raise ValueError('Saved vocabulary mismatch or truth not omitted')
        before = digest(project/'project.json')
        args = [str(a.binary.resolve()), 'expand', '--project', str(project), '--group', 'comparison',
                '--candidates', str(candidate_path), '--provider', str(a.provider), '--max-proposals', '512',
                '--paragraph-context', '--output', str(out/'expansion.json')]
        args += ['--provider-arg='+s for s in a.provider_arg]
        if a.width_guidance:
            args.append('--width-guidance')
        start = time.monotonic()
        proc = subprocess.run(args, capture_output=True, text=True, timeout=330)
        (out/'expansion-stderr.txt').write_text(proc.stderr)
        row = dict(case=execution['case'], source_execution=str(path), commands=[args],
                   project_sha256=before, candidates_sha256=digest(candidate_path),
                   expansion_seconds=time.monotonic()-start, status='error' if proc.returncode else 'ok')
        if not proc.returncode:
            expansion = json.loads((out/'expansion.json').read_text())
            original = json.loads((path.parent/'work/native-result.json').read_text())
            if expansion['initial'] != original:
                raise ValueError('Initial physical report changed')
            rank_args = [str(a.binary.resolve()), 'context-rank', '--project', str(project), '--group', 'comparison',
                         '--candidates', str(out/'expansion.json'), '--provider', str(a.context_python),
                         '--provider-arg', str(Path(__file__).with_name('masked_context_provider.py').resolve()),
                         '--provider-arg=--model-dir', '--provider-arg', str(a.context_model),
                         '--paragraph-context', '--word-normalization', '--output', str(out/'ranking.json')]
            row['commands'].append(rank_args)
            ranked = subprocess.run(rank_args, capture_output=True, text=True, timeout=330)
            (out/'ranking-stderr.txt').write_text(ranked.stderr)
            row['status'] = 'ranking_error' if ranked.returncode else 'ok'
            # Only now are labels used to grade generated candidates.
            truth = labels[case['truth_id']]
            generated = [c for c in expansion['candidates'] if c['id'] in expansion['added_candidate_ids']]
            truth_ids = {c['id'] for c in generated if c['canonical_name'] == truth}
            compatible = set(expansion['compatible_added_candidate_ids'])
            row.update(truth=truth, generated=len(generated), compatible_generated=len(compatible),
                       truth_proposed=bool(truth_ids), truth_proposed_and_compatible=bool(truth_ids & compatible),
                       original_compatible=original['compatible_candidates'],
                       expanded_compatible=expansion['expanded']['compatible_candidates'])
            if not ranked.returncode:
                ranking = json.loads((out/'ranking.json').read_text())
                if ranking['physical'] != expansion['expanded']:
                    raise ValueError('Context ranking changed expanded physical evidence')
                groups = ranking['ranked_groups']
                row['truth_unique_first'] = bool(groups and len(groups[0]) == 1 and groups[0][0] in truth_ids)
                row['truth_rank'] = next(([1 + sum(map(len, groups[:i])), sum(map(len, groups[:i+1]))]
                                          for i, g in enumerate(groups) if truth_ids.intersection(g)), None)
        if digest(project/'project.json') != before:
            raise ValueError('Read-only source project changed')
        rows.append(row)
        (out/'evaluation.json').write_text(json.dumps(row, indent=2)+'\n')
        print(execution['case'], row['status'], {k: row.get(k) for k in ['truth_proposed', 'truth_unique_first', 'expanded_compatible']}, flush=True)
    if digest(a.binary) != binary_hash:
        raise ValueError('Binary changed during evaluation')
    summary = dict(cases=len(rows), successful=sum(r['status']=='ok' for r in rows),
                   truth_proposed=sum(r.get('truth_proposed', False) for r in rows),
                   truth_proposed_and_compatible=sum(r.get('truth_proposed_and_compatible', False) for r in rows),
                   truth_unique_first=sum(r.get('truth_unique_first', False) for r in rows))
    (a.output/'summary.json').write_text(json.dumps(dict(summary=summary, rows=rows, binary_sha256=binary_hash,
        comparison_sha256=digest(a.comparison/'comparison.json'), suite_sha256=digest(a.suite),
        script_sha256=digest(Path(__file__)),
        interpretation='Known controlled cases reused as development data. Truth omitted from original vocabulary; labels only grade outputs. Expanded candidate sets differ from the closed-vocabulary benchmark and cannot be reported as its matched external comparison. Proposal failures and ranking failures remain explicit; bounded search cannot certify absence.'), indent=2)+'\n')
    print(summary)


if __name__ == '__main__':
    main()
