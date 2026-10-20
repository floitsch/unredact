# Copyright (C) 2026 Toit contributors.
"""Offline T5 complete-span scorer for Rust context-rank.

Scores replacement tokens AND their closing sentinel, conditional on the visible
context and opening sentinel. No candidate length is supplied to the encoder.
Use context-rank --sequence-score to preserve these complete sequence log sums.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import sys
import unicodedata

from fetch_context_model import digest
from span_proposal_provider import LIMIT, prepare as prepare_contexts, verify_model


def prepare(request, tokenizer, context_tokens=64):
    if request.get('schema_version') != 1 or request.get('task') != 'score_context_candidates':
        raise ValueError('Expected context-score protocol version 1')
    contexts, _, _ = prepare_contexts(dict(schema_version=1, task='propose_context_candidates',
        surface_kind='canonical', contexts=request['contexts'],
        limits=dict(max_proposals=4096, max_text_bytes=512)), tokenizer, context_tokens)
    items = request['items']
    if not isinstance(items, list) or not 1 <= len(items) <= 8192:
        raise ValueError('Expected 1–8192 candidate/occurrence items')
    result, seen = [], set()
    for item in items:
        if not isinstance(item.get('id'), str) or not item['id'] or not isinstance(item.get('target'), str):
            raise ValueError('Invalid candidate or target ID')
        key = (item['id'], item['target'])
        if key in seen or item['target'] not in contexts:
            raise ValueError('Duplicate item or unknown target')
        seen.add(key)
        text = item['text']
        if (not isinstance(text, str) or not text.strip() or len(text.encode()) > 512
                or any(unicodedata.category(c) == 'Cc' for c in text)):
            raise ValueError('Invalid candidate text')
        tokens = tokenizer.encode(text, add_special_tokens=False)
        if not 1 <= len(tokens) <= 64 or any(t in tokenizer.all_special_ids for t in tokens):
            raise ValueError('Candidate needs 1–64 known, non-special tokens')
        result.append(dict(id=item['id'], target=item['target'], tokens=tokens))
    return result, contexts


def decoder_sequence(tokens, tokenizer):
    # Logits at position 0 predict the opening sentinel; omit that common
    # context-only constant. Each later position sees only the candidate prefix.
    first = tokenizer.convert_tokens_to_ids('<extra_id_0>')
    end = tokenizer.convert_tokens_to_ids('<extra_id_1>')
    return [tokenizer.pad_token_id, first, *tokens], [*tokens, end]


def score_items(items, contexts, tokenizer, model, torch, batch_size=16):
    from transformers.modeling_outputs import BaseModelOutput
    grouped = {}
    for i, item in enumerate(items):
        key = (tuple(contexts[item['target']]['input_ids']), tuple(item['tokens']))
        grouped.setdefault(key, []).append(i)
    by_context = {}
    for (context, tokens), consumers in sorted(grouped.items()):
        by_context.setdefault(context, []).append((tokens, consumers))
    with torch.inference_mode():
        for context, entries in by_context.items():
            encoder_ids = torch.tensor([context])
            encoder_attention = torch.ones_like(encoder_ids)
            hidden = model.encoder(input_ids=encoder_ids, attention_mask=encoder_attention).last_hidden_state
            for start in range(0, len(entries), batch_size):
                batch = entries[start:start + batch_size]
                decoded = [decoder_sequence(tokens, tokenizer) for tokens, _ in batch]
                width = max(len(ids) for ids, _ in decoded)
                ids = torch.full((len(batch), width), tokenizer.pad_token_id, dtype=torch.long)
                attention = torch.zeros_like(ids)
                for row, (sequence, _) in enumerate(decoded):
                    ids[row, :len(sequence)] = torch.tensor(sequence)
                    # T5's initial PAD is a meaningful decoder-start token.
                    attention[row, :len(sequence)] = 1
                logits = model(encoder_outputs=BaseModelOutput(last_hidden_state=hidden.expand(len(batch), -1, -1)),
                    attention_mask=encoder_attention.expand(len(batch), -1),
                    decoder_input_ids=ids, decoder_attention_mask=attention, use_cache=False).logits
                log_probs = logits.log_softmax(dim=-1)
                for row, ((_, predictions), (_, consumers)) in enumerate(zip(decoded, batch)):
                    values = log_probs[row, torch.arange(1, len(predictions) + 1), torch.tensor(predictions)]
                    parts = values.tolist()
                    if any(not math.isfinite(v) or v > 0 for v in parts):
                        raise ValueError('Invalid model token log score')
                    for i in consumers:
                        items[i]['token_log_score_sum'] = sum(parts)
                        items[i]['token_count'] = len(parts)
                        items[i]['token_log_scores'] = parts
    return dict(unique_contexts=len(by_context), unique_sequences=len(grouped))


def run(request, args):
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    os.environ['TOKENIZERS_PARALLELISM'] = 'false'
    import torch
    import transformers
    from transformers import T5ForConditionalGeneration, T5TokenizerFast
    torch.set_num_threads(args.threads)
    provenance = verify_model(args.model_dir)
    tokenizer = T5TokenizerFast.from_pretrained(args.model_dir, local_files_only=True)
    model = T5ForConditionalGeneration.from_pretrained(args.model_dir, local_files_only=True,
                                                       use_safetensors=True).eval()
    if verify_model(args.model_dir) != provenance:
        raise ValueError('Model changed during loading')
    adapter = None
    if getattr(args, 'adapter', None):
        from span_adapter import load
        adapter = load(model, args.adapter, provenance)
    items, contexts = prepare(request, tokenizer, args.context_tokens)
    stats = score_items(items, contexts, tokenizer, model, torch, args.batch_size)
    scores = [{k: item[k] for k in ('id', 'target', 'token_log_score_sum', 'token_count')} for item in items]
    files = ['span_context_provider.py', 'span_proposal_provider.py', 'ollama_provider.py',
             'fetch_span_model.py', 'fetch_context_model.py']
    result = dict(schema_version=1, scores=scores, provenance=provenance,
        method='t5_candidate_and_closing_sentinel_given_opening_sentinel',
        interpretation='Complete span log score with termination, not a posterior probability of the hidden text. Token counts include the closing sentinel. Use sequence-score to avoid length normalization.',
        contexts_used=contexts, diagnostics=items, forward_states=stats,
        options=dict(threads=args.threads, batch_size=args.batch_size, context_tokens=args.context_tokens),
        runtime=dict(torch=torch.__version__, transformers=transformers.__version__),
        request_sha256=hashlib.sha256(json.dumps(request, sort_keys=True).encode()).hexdigest(),
        source_hashes={name: digest(Path(__file__).with_name(name)) for name in files})
    if adapter is not None:
        result['domain_adapter'] = adapter
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model-dir', type=Path, required=True)
    parser.add_argument('--adapter', type=Path, help='Optional verified local domain adapter directory')
    parser.add_argument('--threads', type=int, default=4)
    parser.add_argument('--batch-size', type=int, default=16)
    parser.add_argument('--context-tokens', type=int, default=64)
    args = parser.parse_args()
    if not 1 <= args.threads <= 32 or not 1 <= args.batch_size <= 64 or not 1 <= args.context_tokens <= 192:
        parser.error('Invalid inference limits')
    raw = sys.stdin.buffer.read(LIMIT + 1)
    if len(raw) > LIMIT:
        parser.error('Request exceeds 8 MB')
    try:
        result = json.dumps(run(json.loads(raw), args), ensure_ascii=False, allow_nan=False)
        if len(result.encode()) > LIMIT:
            raise ValueError('Response exceeds 8 MB')
    except (ValueError, KeyError, TypeError, OSError) as error:
        parser.exit(1, f'Span context provider: {error}\n')
    print(result)


if __name__ == '__main__':
    main()
