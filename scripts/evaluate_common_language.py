# Copyright (C) 2026 Toit contributors.
"""Give saved geometric candidate sets identical T5 sequence scores.

This is a common-language control, not a measurement of upstream Claude features.
Only tools exposing compatible sets can participate without inventing a cutoff.
"""
import argparse
import json
import os
from pathlib import Path

from fetch_context_model import digest
from span_context_provider import prepare, score_items, verify_model


def truth_rank(scores, truth):
    if truth not in scores:
        return None
    value = scores[truth]
    better = sum(s > value + 1e-9 for s in scores.values())
    tied = sum(abs(s - value) <= 1e-9 for s in scores.values())
    return [better + 1, better + tied]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest', type=Path, required=True)
    p.add_argument('--model-dir', type=Path, default=Path('runs/span-model-001/t5-base'))
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    manifest = json.loads(args.manifest.read_text())
    args.output.mkdir(parents=True, exist_ok=False)
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    os.environ['TOKENIZERS_PARALLELISM'] = 'false'
    import torch
    import transformers
    from transformers import T5TokenizerFast, T5ForConditionalGeneration
    torch.set_num_threads(4)
    provenance = verify_model(args.model_dir)
    tokenizer = T5TokenizerFast.from_pretrained(args.model_dir, local_files_only=True)
    model = T5ForConditionalGeneration.from_pretrained(args.model_dir, local_files_only=True,
                                                       use_safetensors=True).eval()
    rows, summary = [], {}
    for index, case in enumerate(manifest['cases']):
        inputs = [case['control'], case['candidates'], case['external_comparison'], case['local_sequence_report']]
        hashes = {name: digest(Path(name)) for name in inputs}
        context = json.loads(Path(case['control']).read_text())
        if len(context['contexts']) != 1:
            raise ValueError('This control requires one occurrence per case')
        target = next(iter(context['contexts']))
        candidates = json.loads(Path(case['candidates']).read_text())['candidates']
        if any(len(c['surfaces']) != 1 for c in candidates):
            raise ValueError('This control requires single-surface candidates')
        candidate_ids = {c['id'] for c in candidates}
        request = dict(schema_version=1, task='score_context_candidates', contexts=context['contexts'],
            items=[dict(id=c['id'], target=target, text=c['surfaces'][0]['text']) for c in candidates])
        prepared, used = prepare(request, tokenizer, 64)
        stats = score_items(prepared, used, tokenizer, model, torch, 16)
        scores = {i['id']: i['token_log_score_sum'] for i in prepared}
        local_allowed = {c['id'] for c in context['physical']['candidates'] if c['compatible']}
        sets = {'language_only': candidate_ids, 'local_geometry': local_allowed}
        external = json.loads(Path(case['external_comparison']).read_text())
        executions = [e for e in external['executions'] if e['case'] == case['id'] and e['adapter'].startswith('alex-')]
        if not executions:
            raise ValueError('No Alex executions for this case')
        for e in executions:
            if e['error'] or e['result']['status'] != 'ok' or e['input_sha256'] != case['input_sha256']:
                raise ValueError('External execution failed or used different inputs')
            if e['result']['compatible'] is None:
                raise ValueError('External compatible set is not exposed')
            allowed = set(e['result']['compatible'])
            if not allowed.issubset(candidate_ids):
                raise ValueError('External candidate IDs differ')
            sets[e['adapter']] = allowed
        # Truth labels only grade the fully computed, shared language scores.
        truth = case['truth_ids'][0]
        results = {name: dict(truth_rank=truth_rank({k: scores[k] for k in allowed}, truth),
                             compatible=len(allowed), truth_compatible=truth in allowed)
                   for name, allowed in sets.items()}
        local_sequence = json.loads(Path(case['local_sequence_report']).read_text())
        if local_sequence['physical'] != context['physical'] or local_sequence['contexts'] != context['contexts']:
            raise ValueError('Local sequence report differs from control')
        expected = {r['id']: r['mean_sequence_log_score'] for r in local_sequence['ranked']}
        if set(expected) != local_allowed or any(abs(expected[k] - scores[k]) > 1e-4 for k in expected):
            raise ValueError('Shared scores disagree with direct Rust/provider execution')
        if truth_rank(expected, truth) != results['local_geometry']['truth_rank']:
            raise ValueError('Batch numerical differences changed local truth ranking')
        if any(digest(Path(name)) != sha for name, sha in hashes.items()):
            raise ValueError('Saved comparison input changed')
        for name, result in results.items():
            totals = summary.setdefault(case['cohort'] + '::' + name, dict(cases=0, unique_first=0, truth_compatible=0))
            totals['cases'] += 1
            totals['unique_first'] += result['truth_rank'] == [1, 1]
            totals['truth_compatible'] += result['truth_compatible']
        row = dict(case=case['id'], cohort=case['cohort'], input_hashes=hashes,
                   request=request, scores=prepared, contexts_used=used, forward_states=stats, results=results)
        (args.output / f'case-{index:03}.json').write_text(json.dumps(row, indent=2) + '\n')
        rows.append(dict(case=case['id'], cohort=case['cohort'], results=results))
        print(case['cohort'], case['id'], {k: v['truth_rank'] for k, v in results.items()}, flush=True)
    if verify_model(args.model_dir) != provenance:
        raise ValueError('Model changed during evaluation')
    (args.output / 'summary.json').write_text(json.dumps(dict(summary=summary, rows=rows,
        model=provenance, runtime=dict(torch=torch.__version__, transformers=transformers.__version__),
        manifest_sha256=digest(args.manifest), script_sha256=digest(Path(__file__)),
        provider_sha256=digest(Path(__file__).with_name('span_context_provider.py')),
        interpretation='Reused closed-vocabulary cases. One shared T5/context score per candidate, applied to each saved geometric compatible set. Morten exposes no compatible set and is excluded; no threshold is invented. This is not upstream Claude, upstream OCR context selection, or a new end-to-end baseline benchmark.'), indent=2) + '\n')
    print(json.dumps(summary), flush=True)


if __name__ == '__main__':
    main()
