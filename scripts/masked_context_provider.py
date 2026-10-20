# Copyright (C) 2026 Toit contributors.
"""Offline BERT contextual scorer for the Rust context-rank JSON protocol.

For each candidate, mask all its WordPieces, then reveal them left-to-right.
Each token is scored without seeing later candidate tokens. Both visible
contexts are retained. This is a length-conditioned score, not a normalized
probability over replacement phrases. The Rust caller averages token log scores.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_model(root):
    provenance = json.loads((root / 'provenance.json').read_text())
    names = ['config.json', 'tokenizer.json', 'tokenizer_config.json',
             'vocab.txt', 'model.safetensors']
    hashes = {}
    for name in names:
        hashes[name] = digest(root / name)
        if hashes[name] != provenance['files'][name]['sha256']:
            raise ValueError(f'Model artifact hash mismatch: {name}')
    return {'repository': provenance['repository'], 'revision': provenance['revision'],
            'files': hashes, 'provenance_sha256': digest(root / 'provenance.json')}


def prepare(request, tokenizer, context_tokens=64):
    if request.get('schema_version') != 1 or request.get('task') != 'score_context_candidates':
        raise ValueError('Expected context-score protocol version 1')
    contexts, items = request['contexts'], request['items']
    if not isinstance(items, list) or not 1 <= len(items) <= 8192:
        raise ValueError('Expected 1-8192 candidate/occurrence items')
    encoded, contexts_used = {}, {}
    for key, context in contexts.items():
        left = tokenizer.encode(context['left'], add_special_tokens=False)
        right = tokenizer.encode(context['right'], add_special_tokens=False)
        reserved = set(tokenizer.all_special_ids) - {getattr(tokenizer, 'unk_token_id', None)}
        if any(t in reserved for t in left + right):
            raise ValueError('Context contains reserved special tokens')
        if not left and not right:
            raise ValueError('Empty context')
        encoded[key] = (tuple(left[-context_tokens:]), tuple(right[:context_tokens]))
        contexts_used[key] = dict(left_token_ids=list(encoded[key][0]),
                                  right_token_ids=list(encoded[key][1]),
                                  left_truncated=len(left) > context_tokens,
                                  right_truncated=len(right) > context_tokens)
    seen, prepared = set(), []
    for item in items:
        key = (item['id'], item['target'])
        if key in seen or item['target'] not in encoded:
            raise ValueError('Duplicate item or unknown target')
        seen.add(key)
        if not isinstance(item['text'], str) or not 1 <= len(item['text'].encode()) <= 512:
            raise ValueError('Invalid candidate text')
        tokens = tuple(tokenizer.encode(item['text'], add_special_tokens=False))
        if not 1 <= len(tokens) <= 32 or any(t in tokenizer.all_special_ids for t in tokens):
            raise ValueError('Candidate needs 1-32 known, non-special WordPieces')
        left, right = encoded[item['target']]
        prepared.append(dict(id=item['id'], target=item['target'], tokens=tokens,
                             left=left, right=right, token_log_score_sum=0.0))
    return prepared, contexts_used


def states_at_step(items, step):
    """Deduplicate identical model inputs; sort to avoid candidate-order effects."""
    states = {}
    for index, item in enumerate(items):
        tokens = item['tokens']
        if step < len(tokens):
            key = (item['left'], item['right'], len(tokens), tokens[:step])
            states.setdefault(key, []).append((index, tokens[step]))
    return sorted(states.items())


def score_items(items, tokenizer, model, torch, batch_size):
    state_count = 0
    with torch.inference_mode():
        for step in range(max(len(item['tokens']) for item in items)):
            states = states_at_step(items, step)
            state_count += len(states)
            for offset in range(0, len(states), batch_size):
                batch = states[offset:offset + batch_size]
                sequences, positions = [], []
                for (left, right, length, prefix), _ in batch:
                    sequences.append([tokenizer.cls_token_id, *left, *prefix,
                        *([tokenizer.mask_token_id] * (length - step)),
                        *right, tokenizer.sep_token_id])
                    positions.append(1 + len(left) + step)
                width = max(map(len, sequences))
                ids = torch.full((len(batch), width), tokenizer.pad_token_id, dtype=torch.long)
                attention = torch.zeros_like(ids)
                for row, sequence in enumerate(sequences):
                    ids[row, :len(sequence)] = torch.tensor(sequence)
                    attention[row, :len(sequence)] = 1
                # Apply the prediction head only at the masked position; the
                # full sequence still participates in bidirectional attention.
                hidden = model.bert(input_ids=ids, attention_mask=attention).last_hidden_state
                selected = hidden[torch.arange(len(batch)), torch.tensor(positions)]
                log_probs = model.cls(selected).log_softmax(dim=-1)
                for row, (_, consumers) in enumerate(batch):
                    for index, token in consumers:
                        items[index]['token_log_score_sum'] += float(log_probs[row, token])
    return state_count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model-dir', type=Path, required=True)
    parser.add_argument('--threads', type=int, default=4)
    parser.add_argument('--batch-size', type=int, default=16)
    parser.add_argument('--context-tokens', type=int, default=64)
    args = parser.parse_args()
    if not 1 <= args.threads <= 32 or not 1 <= args.batch_size <= 64 or not 1 <= args.context_tokens <= 192:
        parser.error('Invalid thread, batch or context limit')
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    os.environ['TOKENIZERS_PARALLELISM'] = 'false'
    import torch
    import transformers
    from transformers import BertForMaskedLM, BertTokenizerFast
    torch.set_num_threads(args.threads)
    provenance = verify_model(args.model_dir)
    tokenizer = BertTokenizerFast.from_pretrained(args.model_dir, local_files_only=True)
    model = BertForMaskedLM.from_pretrained(args.model_dir, local_files_only=True,
        use_safetensors=True, attn_implementation='eager').eval()
    if verify_model(args.model_dir) != provenance:
        raise ValueError('Model artifacts changed during loading')
    request = json.load(sys.stdin)
    items, contexts_used = prepare(request, tokenizer, args.context_tokens)
    state_count = score_items(items, tokenizer, model, torch, args.batch_size)
    scores = [dict(id=i['id'], target=i['target'], token_log_score_sum=i['token_log_score_sum'],
                   token_count=len(i['tokens'])) for i in items]
    json.dump(dict(schema_version=1, scores=scores, provenance=provenance,
        method='left_to_right_reveal_all_candidate_wordpieces_initially_masked',
        interpretation='Length-conditioned contextual score; not a normalized phrase probability or evidence of hidden identity.',
        contexts_used=contexts_used, unique_forward_states=state_count,
        options=dict(threads=args.threads, batch_size=args.batch_size, context_tokens=args.context_tokens),
        runtime=dict(torch=torch.__version__, transformers=transformers.__version__),
        provider_sha256=digest(Path(__file__))), sys.stdout, allow_nan=False)


if __name__ == '__main__':
    main()
