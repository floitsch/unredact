# Copyright (C) 2026 Toit contributors.
"""Common local-T5 scores on complete, blinded multidocument development suites.

All 300 candidates receive identical language scores for every geometric filter.
Rust weight-0/weight-1 rankings are checked by replay through the real CLI. No
truth label is sent to the scorer, and no external compatibility cutoff is added.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys

from evaluate_common_language import truth_rank
from span_context_provider import prepare, score_items, verify_model
from summarize_multidoc_benchmark import cluster_interval


LOCAL = {'local-profile-cost': 0, 'local-uniform-padding': 3}
TRACKS = ['language-only', 'local-profile-cost-weight-0', 'local-profile-cost-weight-1',
          'local-uniform-padding-weight-0', 'local-uniform-padding-weight-1',
          'alex-matched-catalog-wide-adaptation', 'morten-matched-catalog-adaptation']


def load(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def verify(files):
    for path, expected in files.items():
        if digest(path) != expected:
            raise ValueError(f'Frozen file changed: {path}')


def grade(scores, allowed, truth, costs=None, weight=0):
    """Absent labels never become a successful prediction; empty is only a proxy."""
    if allowed is None:
        return dict(status='not_available', reason='No compatibility set exposed')
    allowed = set(allowed)
    if not allowed <= scores.keys():
        raise ValueError('Unknown candidate in geometric filter')
    ranked = {k: scores[k] - weight * (costs[k] if costs else 0) for k in allowed}
    if any(not math.isfinite(x) for x in ranked.values()):
        raise ValueError('Nonfinite combined score')
    interval = truth_rank(ranked, truth) if truth is not None else None
    return dict(status='ok', truth_rank=interval, unique_correct_first=interval == [1, 1],
                truth_compatible=truth in allowed if truth is not None else None,
                compatible=len(allowed), empty_set_proxy=not allowed,
                single_wrong_candidate=truth is None and len(allowed) == 1)


def verify_ranked(ranked, allowed, scores, costs, weight):
    by_id = {c['id']: c for c in ranked}
    if len(by_id) != len(ranked) or set(by_id) != set(allowed):
        raise ValueError('CLI ranked candidate mismatch')
    for identity, candidate in by_id.items():
        # Rust omits combined_score at weight zero: the sequence score is used.
        actual = candidate['combined_score'] if weight else candidate['mean_sequence_log_score']
        if (abs(candidate['mean_sequence_log_score'] - scores[identity]) > 1e-9 or
                abs(actual - (scores[identity] - weight * costs[identity])) > 1e-9):
            raise ValueError('CLI combined score mismatch')


def summarize(rows):
    groups = defaultdict(list)
    for row in rows:
        for breakdown, value in [('all', 'all'), ('document', row['document']),
                                 ('kind', row['kind']), ('word_count', str(row['word_count']))]:
            groups[row['availability'], breakdown, value].append(row)
    metrics, paired = [], []
    for (availability, breakdown, value), selected in sorted(groups.items()):
        doc_means = {}
        for track in TRACKS:
            values = [r['tracks'][track] for r in selected]
            ok = [v for v in values if v['status'] == 'ok']
            by_doc = defaultdict(list)
            for r, v in zip(selected, values):
                by_doc[r['source_group']].append(float(v.get('unique_correct_first', False)))
            means = {d: statistics.mean(v) for d, v in sorted(by_doc.items())}
            doc_means[track] = means
            # An unavailable interface has no estimated accuracy, including in
            # paired comparisons. Operational failures remain in denominators.
            exposed = any(v['status'] != 'not_available' for v in values)
            report = dict(availability=availability, breakdown=breakdown, value=value,
                          track=track, cases=len(selected), statuses=dict(Counter(v['status'] for v in values)))
            if availability == 'present':
                report.update(unique_correct_first=sum(v['unique_correct_first'] for v in ok) if exposed else None,
                              truth_compatible=sum(v['truth_compatible'] for v in ok) if exposed else None,
                              top5_guaranteed=sum(v['truth_rank'] is not None and v['truth_rank'][1] <= 5 for v in ok) if exposed else None,
                              document_macro_first=statistics.mean(means.values()) if exposed else None,
                              document_95_interval=cluster_interval(list(means.values())) if exposed else None)
            else:
                report.update(empty_set_proxy=sum(v['empty_set_proxy'] for v in ok) if exposed else None,
                              single_wrong_candidate=sum(v['single_wrong_candidate'] for v in ok) if exposed else None)
            metrics.append(report)
        if availability == 'present' and breakdown == 'all':
            available = [t for t in TRACKS if any(r['tracks'][t]['status'] == 'ok' for r in selected)]
            for i, first in enumerate(available):
                for second in available[i + 1:]:
                    differences = [doc_means[first][d] - doc_means[second][d] for d in sorted(doc_means[first])]
                    paired.append(dict(first=first, second=second, cases=len(selected), documents=len(differences),
                                       document_macro_difference=statistics.mean(differences),
                                       document_95_interval=cluster_interval(differences)))
    return dict(metrics=metrics, paired=paired)


def run(command, stderr_path):
    result = subprocess.run(list(map(str, command)), capture_output=True, text=True, timeout=330)
    stderr_path.write_text(result.stderr)
    if result.returncode:
        raise RuntimeError(f'CLI exit {result.returncode}: {result.stderr[-2000:]}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--capture', type=Path)
    args = parser.parse_args()
    if args.capture:
        request = json.load(sys.stdin)
        if args.capture.exists():
            raise ValueError('Expected one bounded protocol request')
        write(args.capture, request)
        print(json.dumps(dict(schema_version=1, scores=[dict(id=i['id'], target=i['target'],
              token_count=1, token_log_score_sum=0.) for i in request['items']], method='UNSCORED_PROTOCOL_CAPTURE')))
        return
    if not args.plan or not args.output:
        parser.error('Supply --plan and --output')
    plan = load(args.plan)
    if plan['split'] != 'development' or plan['condition'] != 'opaque' or plan['weights'] != [0, 1]:
        raise ValueError('This evaluation is fixed to development opaque cases, weights 0 and 1')
    root, benchmark = Path(plan['geometry_root']), Path(plan['benchmark'])
    files = load(root / 'freeze.json')
    verify(files)
    geometry_plan = load(root / 'plan.json')
    manifest = load(benchmark / 'manifest.json')
    if manifest['label_review'] != 'verified':
        raise ValueError('Labels require review')
    suites, reports = {}, {}
    artifacts = [args.plan, Path(__file__), root / 'freeze.json']
    for availability, directory in geometry_plan['availability_runs'].items():
        suite_path = benchmark / f'development-opaque-{availability}.json'
        report_path = root / directory / 'comparison.json'
        suite, report = load(suite_path), load(report_path)  # Requires terminal comparison reports.
        if len(suite['cases']) != plan['cases_per_suite'] or len(suite['cases']) != 96:
            raise ValueError('Full 96-case suite required')
        if report['suite_sha256'] != digest(suite_path) or any(p.get('error') for p in report['provenance']):
            raise ValueError('Geometry provenance mismatch')
        expected = {(c['id'], t, 0) for c in suite['cases'] for t in geometry_plan['adapters']}
        actual = {(e['case'], e['adapter'], e['repetition']) for e in report['executions']}
        if actual != expected or len(report['executions']) != len(expected):
            raise ValueError('Incomplete/duplicate geometry executions')
        suites[availability] = suite
        reports[availability] = {(e['case'], e['adapter']): e for e in report['executions']}
        artifacts += [suite_path, report_path]
        for path in (root / directory).glob('*/work/project/project.json'):
            artifacts += [path, path.parent.parent / 'candidates.json', path.parent.parent / 'native-result.json']
            for page in load(path)['pages']:
                artifacts.append(path.parent / page['image'])
    script_dir = Path(__file__).resolve().parent
    for name in ['evaluate_common_language.py', 'span_context_provider.py', 'span_proposal_provider.py',
                 'fetch_context_model.py', 'fetch_span_model.py', 'ollama_provider.py',
                 'summarize_multidoc_benchmark.py', 'evaluate_bounded_controls.py', 'evaluate_span_ranking.py']:
        artifacts.append(script_dir / name)
    files.update({str(p.resolve()): digest(p) for p in artifacts})
    model_dir = Path(plan['model_dir'])
    provenance = verify_model(model_dir)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    write(out / 'freeze.json', dict(files=files, model=provenance, plan=plan))
    os.environ.update(HF_HUB_OFFLINE='1', TRANSFORMERS_OFFLINE='1', TOKENIZERS_PARALLELISM='false')
    import torch
    import transformers
    from transformers import T5TokenizerFast, T5ForConditionalGeneration
    torch.set_num_threads(plan['threads'])
    tokenizer = T5TokenizerFast.from_pretrained(model_dir, local_files_only=True)
    model = T5ForConditionalGeneration.from_pretrained(model_dir, local_files_only=True, use_safetensors=True).eval()
    write(out / 'runtime.json', dict(torch=torch.__version__, transformers=transformers.__version__))
    binary = Path('target/release/unredact').resolve()
    provider_python = Path('runs/baseline-venv/bin/python').absolute()
    label_index = {c['id']: r for r in load(benchmark / 'controller/labels.json')['labels'] for c in r['cases']}
    rows = []
    for availability in ['present', 'absent']:
        for index, case in enumerate(suites[availability]['cases']):
            # Opaque case IDs are also used for output paths passed to providers.
            directory = out / case['id']
            directory.mkdir()
            label = label_index[case['id']]
            row = dict(case=case['id'], availability=availability, document=label['document'],
                       source_group=label['source_group'], kind=label['kind'], word_count=label['count'], tracks={})
            entries = {t: reports[availability][case['id'], t] for t in geometry_plan['adapters']}
            if len({e['input_sha256'] for e in entries.values()}) != 1:
                raise ValueError('Different geometric inputs')
            paths, physicals, contexts = {}, {}, None
            failed = {}
            for local, tool in LOCAL.items():
                entry = entries[local]
                if entry['error'] or entry['result']['status'] != 'ok':
                    failed[local] = entry['error'] or entry['result'].get('reason', 'Geometry failed')
                    continue
                work = root / geometry_plan['availability_runs'][availability] / f'case-{index:03}-repeat-00-tool-{tool:02}' / 'work'
                physicals[local] = load(work / 'native-result.json')
                paths[local] = work
                capture = directory / f'{local}-request.json'
                unscored = directory / f'{local}-UNSCORED.json'
                run([binary, 'context-rank', '--project', work / 'project', '--group', 'comparison',
                     '--paragraph-context', '--sequence-score', '--candidates', work / 'candidates.json',
                     '--provider', provider_python, '--provider-arg', Path(__file__).resolve(),
                     '--provider-arg=--capture', '--provider-arg', capture, '--output', unscored],
                    directory / f'{local}-capture.stderr')
                captured = load(unscored)
                if captured['physical'] != physicals[local]:
                    raise ValueError('Geometry changed during context capture')
                if contexts is not None and captured['contexts'] != contexts:
                    raise ValueError('Local modes selected different contexts')
                contexts = captured['contexts']
            if contexts is None:
                # No successful context pipeline: keep the entire planned case.
                row['tracks'] = {t: dict(status='error', reason='No local context available') for t in TRACKS}
            else:
                if len(contexts) != 1:
                    raise ValueError('Expected one target per case')
                target = next(iter(contexts))
                if not target.startswith('t-') or len(target) != 66:
                    raise ValueError('Nonopaque target ID')
                candidates = case['input']['candidates']
                if len(candidates) != 300 or any(len(c['surfaces']) != 1 for c in candidates):
                    raise ValueError('Expected equal 300-candidate single-surface controls')
                request = dict(schema_version=1, task='score_context_candidates', contexts=contexts,
                               items=[dict(id=c['id'], target=target, text=c['surfaces'][0]['text']) for c in candidates])
                prepared, used = prepare(request, tokenizer, plan['context_tokens'])
                stats = score_items(prepared, used, tokenizer, model, torch, plan['batch_size'])
                cached = directory / 'scores.json'
                write(cached, dict(request=request, scores=prepared, contexts_used=used, forward_states=stats, model=provenance))
                scores = {c['id']: c['token_log_score_sum'] for c in prepared}
                # Truth is used only after all candidate scores are computed.
                truth = case['truth_id'] if availability == 'present' else None
                if (case['truth_id'] in scores) != (availability == 'present'):
                    raise ValueError('Candidate availability mismatch')
                row['tracks']['language-only'] = grade(scores, scores.keys(), truth)
                for local in LOCAL:
                    if local in failed:
                        for weight in plan['weights']:
                            row['tracks'][f'{local}-weight-{weight}'] = dict(status='error', reason=failed[local])
                        continue
                    physical = physicals[local]
                    allowed = {c['id'] for c in physical['candidates'] if c['compatible']}
                    if allowed != set(entries[local]['result']['compatible']):
                        raise ValueError('Native and adapter compatibility differ')
                    costs = {c['id']: c['evidence_score'] / physical['independent_observations'] for c in physical['candidates']}
                    for weight in plan['weights']:
                        work = paths[local]
                        report_path = directory / f'{local}-weight-{weight}.json'
                        run([binary, 'context-rank', '--project', work / 'project', '--group', 'comparison',
                             '--paragraph-context', '--sequence-score', '--physical-weight', weight,
                             '--candidates', work / 'candidates.json', '--provider', provider_python,
                             '--provider-arg', script_dir / 'evaluate_bounded_controls.py', '--provider-arg=--replay',
                             '--provider-arg', cached, '--output', report_path], directory / f'{local}-weight-{weight}.stderr')
                        report = load(report_path)
                        if report['contexts'] != contexts or report['physical'] != physical:
                            raise ValueError('Replay changed contexts/geometry')
                        verify_ranked(report['ranked'], allowed, scores, costs, weight)
                        row['tracks'][f'{local}-weight-{weight}'] = grade(scores, allowed, truth, costs, weight)
                for external in geometry_plan['adapters']:
                    if external in LOCAL:
                        continue
                    e = entries[external]
                    row['tracks'][external] = (dict(status='error', reason=e['error'] or e['result'].get('reason'))
                        if e['error'] or e['result']['status'] != 'ok' else grade(scores, e['result']['compatible'], truth))
            rows.append(row)
            write(directory / 'evaluation.json', row)
            write(out / 'progress.json', dict(completed=len(rows), planned=192, last_case=case['id']))
            print(json.dumps(dict(completed=len(rows), case=case['id'])), flush=True)
    verify(files)
    if verify_model(model_dir) != provenance:
        raise ValueError('Model changed')
    write(out / 'summary.json', dict(schema_version=1, rows=rows, **summarize(rows),
        all_frozen_hashes_verified=True, interpretation=plan['interpretation']))


if __name__ == '__main__':
    main()
