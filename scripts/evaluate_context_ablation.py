# Copyright (C) 2026 Toit contributors.
"""Post-run controls: language alone and cyclically shuffled visible contexts.

Uses completed single-target comparisons. Truth labels only grade saved scores;
they are never included in model requests. No parameters are fitted here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time
from masked_context_provider import prepare, score_items, verify_model, digest


def ranking(scores, truth):
    if truth not in scores:
        return None
    score = scores[truth]
    better = sum(s > score + 1e-9 for s in scores.values())
    tied = sum(abs(s - score) <= 1e-9 for s in scores.values())
    return [better + 1, better + tied]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--comparison', type=Path, required=True)
    parser.add_argument('--suite', type=Path, required=True)
    parser.add_argument('--model-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise ValueError('Output must be new')
    report = json.loads((args.comparison / 'comparison.json').read_text())
    if report['suite_sha256'] != digest(args.suite):
        raise ValueError('Suite hash mismatch')
    cases = {c['id']: c for c in json.loads(args.suite.read_text())['cases']}
    records = []
    for path in sorted(args.comparison.glob('*/execution.json')):
        execution = json.loads(path.read_text())
        saved = path.parent / 'work/context-result.json'
        if not saved.exists() or execution.get('error') or not execution['metrics']['truth_in_vocabulary']:
            continue
        context = json.loads(saved.read_text())
        if len(context['contexts']) != 1:
            raise ValueError('Single-target cases only')
        records.append((execution, context, saved))
    if len(records) < 2 or len({e['case'] for e, _, _ in records}) != len(records):
        raise ValueError('Need at least two distinct successful contextual cases, without repetitions')
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    os.environ['TOKENIZERS_PARALLELISM'] = 'false'
    import torch
    import transformers
    from transformers import BertForMaskedLM, BertTokenizerFast
    torch.set_num_threads(4)
    provenance = verify_model(args.model_dir)
    tokenizer = BertTokenizerFast.from_pretrained(args.model_dir, local_files_only=True)
    model = BertForMaskedLM.from_pretrained(args.model_dir, local_files_only=True,
        use_safetensors=True, attn_implementation='eager').eval()
    rows = []
    for index, (execution, context, saved) in enumerate(records):
        case = cases[execution['case']]
        target = next(iter(context['contexts']))
        candidates = case['input']['candidates']
        if any(len(c['surfaces']) != 1 for c in candidates):
            raise ValueError('Single-surface candidates only')
        items = [dict(id=c['id'], target=target, text=c['surfaces'][0]['text']) for c in candidates]
        other_execution, other_context, _ = records[(index + 1) % len(records)]
        allowed = {c['id'] for c in context['physical']['candidates'] if c['compatible']}
        controls = {}
        for mode, ctx in [('correct', context['contexts'][target]),
                          ('shuffled', next(iter(other_context['contexts'].values())))]:
            request = dict(schema_version=1, task='score_context_candidates',
                contexts={target: ctx}, items=items)
            prepared, used = prepare(request, tokenizer, 64)
            start = time.monotonic()
            states = score_items(prepared, tokenizer, model, torch, 16)
            elapsed = time.monotonic() - start
            scores = {i['id']: i['token_log_score_sum'] / len(i['tokens']) for i in prepared}
            # Labels enter only this evaluation block, after model inference.
            truth = case['truth_id']
            controls[mode] = dict(context=ctx, contexts_used=used, scores=scores,
                truth_rank_all=ranking(scores, truth),
                truth_rank_compatible=ranking({k: v for k, v in scores.items() if k in allowed}, truth),
                unique_forward_states=states, elapsed_seconds=elapsed,
                request_sha256=hashlib.sha256(json.dumps(request, sort_keys=True).encode()).hexdigest())
        rows.append(dict(case=execution['case'], original_context_report=str(saved),
            original_context_sha256=digest(saved), shuffled_context_from=other_execution['case'],
            original_hybrid_rank=execution['metrics']['truth_rank'], controls=controls))
        print(execution['case'], {k: (v['truth_rank_all'], v['truth_rank_compatible'])
            for k, v in controls.items()}, flush=True)
    if verify_model(args.model_dir) != provenance:
        raise ValueError('Model changed during evaluation')
    summary = {f'{mode}_{scope}': sum(r['controls'][mode][f'truth_rank_{scope}'] == [1, 1] for r in rows)
        for mode in ['correct', 'shuffled'] for scope in ['all', 'compatible']}
    result = dict(cases=len(rows), unique_correct_first=summary, rows=rows,
        model=provenance, runtime=dict(torch=torch.__version__, transformers=transformers.__version__),
        comparison_sha256=digest(args.comparison / 'comparison.json'), suite_sha256=digest(args.suite),
        script_sha256=digest(Path(__file__)), provider_sha256=digest(Path(__file__).with_name('masked_context_provider.py')),
        interpretation='Post-run ablation, no fitted parameters. Correct context ranks the entire vocabulary; compatible-only applies the unchanged physical filter. Shuffled context is the next successful case in execution-directory order, cyclically. Public pretraining contamination remains unexcluded. Reused cases are not independent extra trials.')
    args.output.write_text(json.dumps(result, indent=2, allow_nan=False) + '\n')
    print(json.dumps(summary))


if __name__ == '__main__':
    main()
