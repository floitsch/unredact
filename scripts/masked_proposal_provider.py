# Copyright (C) 2026 Toit contributors.
"""Offline bounded WordPiece beam proposals for Rust expand.

Generates text from visible context without receiving the candidate list. Each
piece count is a separate masked hypothesis; future pieces remain masked. The
caller checks proposals against joint physical evidence. No exhaustive-search
or absent-vocabulary guarantee follows from this heuristic.
"""
import argparse
import itertools
import json
import math
import os
from pathlib import Path
import sys

from masked_context_provider import digest, verify_model


def prepare(request, tokenizer, context_tokens=64):
    if request.get('schema_version') != 1 or request.get('task') != 'propose_context_candidates':
        raise ValueError('Expected context-proposal protocol version 1')
    contexts = request['contexts']
    limit = request['limits']['max_proposals']
    byte_limit = request['limits']['max_text_bytes']
    if type(limit) is not int or not 1 <= limit <= 4096 or type(byte_limit) is not int or not 1 <= byte_limit <= 512:
        raise ValueError('Invalid proposal limits')
    if not isinstance(contexts, dict) or not 1 <= len(contexts) <= 64:
        raise ValueError('Expected 1–64 independent contexts')
    if limit < len(contexts):
        raise ValueError('Proposal budget must allow at least one per context')
    encoded, used = {}, {}
    reserved = set(tokenizer.all_special_ids) - {tokenizer.unk_token_id}
    for key, context in sorted(contexts.items()):
        if not isinstance(key, str) or not key:
            raise ValueError('Invalid target ID')
        for side in ['left', 'right']:
            if not isinstance(context[side], str) or len(context[side].encode()) > 4096:
                raise ValueError('Invalid context text')
        left = tokenizer.encode(context['left'], add_special_tokens=False)
        right = tokenizer.encode(context['right'], add_special_tokens=False)
        if any(t in reserved for t in left + right) or not left and not right:
            raise ValueError('Empty context or reserved special token')
        encoded[key] = (tuple(left[-context_tokens:]), tuple(right[:context_tokens]))
        used[key] = dict(left_token_ids=list(encoded[key][0]), right_token_ids=list(encoded[key][1]),
                         left_truncated=len(left) > context_tokens, right_truncated=len(right) > context_tokens)
    return encoded, used, limit, byte_limit


def lexical_ids(tokenizer):
    # Explicit ASCII alphabetic scope, including WordPiece continuations.
    initial, continuation = [], []
    for text, token in sorted(tokenizer.get_vocab().items(), key=lambda kv: kv[1]):
        body = text[2:] if text.startswith('##') else text
        if token not in tokenizer.all_special_ids and body.isascii() and body.isalpha():
            (continuation if text.startswith('##') else initial).append(token)
    if not initial:
        raise ValueError('Tokenizer has no alphabetic word-start tokens')
    return initial, continuation


def balanced(groups):
    """Interleave ranked groups without favoring the first target or length."""
    for row in itertools.zip_longest(*groups):
        yield from (item for item in row if item is not None)


def generate(left, right, tokenizer, predict, max_pieces, max_words, beam_width, byte_limit, single_token_count=None):
    initial, continuation = lexical_ids(tokenizer)
    continuation_set = set(continuation)
    by_length, diagnostics = [], []
    for length in range(1, max_pieces + 1):
        width_limit = single_token_count if length == 1 and single_token_count is not None else beam_width
        # tokens, cumulative log score, whitespace word count
        beam = [((), 0.0, 0)]
        expanded_states = 0
        for step in range(length):
            states = [(left, right, length, tokens) for tokens, _, _ in beam]
            allowed = [initial if not tokens else
                       sorted(continuation + (initial if words < max_words else []))
                       for tokens, _, words in beam]
            choices = predict(states, allowed, width_limit)
            if len(choices) != len(beam):
                raise ValueError('Prediction batch mismatch')
            expanded_states += len(states)
            extensions = []
            for (tokens, score, words), options, ids in zip(beam, choices, allowed):
                valid = set(ids)
                seen = set()
                for token, logp in options:
                    if token not in valid or token in seen or not math.isfinite(logp) or logp > 0:
                        raise ValueError('Invalid predicted token or log score')
                    seen.add(token)
                    extensions.append((tokens + (token,), score + logp, words + (token not in continuation_set)))
            beam = sorted(extensions, key=lambda b: (-b[1], b[0]))[:width_limit]
            if not beam:
                break
        completed = []
        for tokens, score, words in beam:
            if len(tokens) != length:
                continue
            text = tokenizer.decode(tokens, skip_special_tokens=False, clean_up_tokenization_spaces=False).strip()
            # Enforce canonical tokenization; this does not prove lexical validity.
            if (not text or len(text.encode()) > byte_limit or len(text.split()) != words
                    or tuple(tokenizer.encode(text, add_special_tokens=False)) != tokens):
                continue
            completed.append(dict(text=text, tokens=list(tokens), words=words, token_log_score_sum=score))
        by_length.append(completed)
        diagnostics.append(dict(pieces=length, expanded_states=expanded_states, completed=len(completed)))
    seen, result = set(), []
    for candidate in balanced(by_length):
        if candidate['text'] not in seen:
            seen.add(candidate['text'])
            result.append(candidate)
    return result, diagnostics


def make_predictor(tokenizer, model, torch, batch_size):
    def predict(states, allowed, count):
        answers = []
        with torch.inference_mode():
            for start in range(0, len(states), batch_size):
                batch = states[start:start + batch_size]
                sequences, positions = [], []
                for left, right, length, prefix in batch:
                    sequences.append([tokenizer.cls_token_id, *left, *prefix,
                                      *([tokenizer.mask_token_id] * (length - len(prefix))),
                                      *right, tokenizer.sep_token_id])
                    positions.append(1 + len(left) + len(prefix))
                width = max(map(len, sequences))
                ids = torch.full((len(batch), width), tokenizer.pad_token_id, dtype=torch.long)
                attention = torch.zeros_like(ids)
                for row, seq in enumerate(sequences):
                    ids[row, :len(seq)] = torch.tensor(seq)
                    attention[row, :len(seq)] = 1
                hidden = model.bert(input_ids=ids, attention_mask=attention).last_hidden_state
                log_probs = model.cls(hidden[torch.arange(len(batch)), torch.tensor(positions)]).log_softmax(dim=-1)
                for row in range(len(batch)):
                    candidates = allowed[start + row]
                    values = log_probs[row, candidates]
                    order = torch.argsort(values, descending=True, stable=True)[:count]
                    answers.append([(candidates[int(i)], float(values[i])) for i in order])
        return answers
    return predict


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model-dir', type=Path, required=True)
    parser.add_argument('--max-pieces', type=int, default=6)
    parser.add_argument('--max-words', type=int, default=3)
    parser.add_argument('--beam-width', type=int, default=16)
    parser.add_argument('--single-token-count', type=int, default=256,
                        help='Broader single-token recall is cheap: one model forward pass')
    parser.add_argument('--threads', type=int, default=4)
    parser.add_argument('--batch-size', type=int, default=16)
    parser.add_argument('--context-tokens', type=int, default=64)
    parser.add_argument('--case-variants', action='store_true', help='Also test titlecase and uppercase spellings; model itself is uncased')
    args = parser.parse_args()
    if not (1 <= args.max_pieces <= 12 and 1 <= args.max_words <= args.max_pieces
            and 1 <= args.beam_width <= 64 and 1 <= args.threads <= 32
            and 1 <= args.single_token_count <= 4096
            and 1 <= args.batch_size <= 64 and 1 <= args.context_tokens <= 192):
        parser.error('Invalid generation limits')
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
        raise ValueError('Model changed during loading')
    request = json.load(sys.stdin)
    contexts, used, limit, byte_limit = prepare(request, tokenizer, args.context_tokens)
    predict = make_predictor(tokenizer, model, torch, args.batch_size)
    groups, diagnostics = [], {}
    for target, (left, right) in contexts.items():
        candidates, lengths = generate(left, right, tokenizer, predict, args.max_pieces,
                                      args.max_words, args.beam_width, byte_limit, args.single_token_count)
        proposals, seen = [], set()
        for c in candidates:
            variants = [c['text'], c['text'].title(), c['text'].upper()] if args.case_variants else [c['text']]
            for text in variants:
                if text not in seen and len(text.encode()) <= byte_limit:
                    seen.add(text)
                    proposals.append(dict(target=target, text=text))
        groups.append(proposals)
        diagnostics[target] = dict(lengths=lengths, generated=candidates, with_case_variants=len(proposals))
    proposals = list(itertools.islice(balanced(groups), limit))
    json.dump(dict(schema_version=1, proposals=proposals, provenance=provenance,
        contexts_used=used, search=diagnostics, response_limit_reached=sum(map(len, groups)) > limit,
        options={**vars(args), 'model_dir': str(args.model_dir)},
        interpretation='Bounded alphabetic WordPiece beams, balanced across piece counts and occurrences. Future pieces remain masked. Not exhaustive; no absence certificate or calibrated probability. Casing variants, when enabled, are hypotheses.',
        runtime=dict(torch=torch.__version__, transformers=transformers.__version__),
        provider_sha256=digest(Path(__file__)), shared_provider_sha256=digest(Path(__file__).with_name('masked_context_provider.py'))),
        sys.stdout, allow_nan=False)


if __name__ == '__main__':
    main()
