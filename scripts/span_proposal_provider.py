# Copyright (C) 2026 Toit contributors.
"""Offline T5 missing-span proposals for Rust expand.

Both visible sides condition a variable-length decoder. Only spans ending with
the closing sentinel become proposals; truncated beams never become answers.
The Rust engine evaluates every proposal against all linked physical evidence.
Beam search is bounded and cannot certify that the true text is absent.
"""
import argparse
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import sys
import unicodedata

from fetch_context_model import digest
from fetch_span_model import EXPECTED, REPOSITORY, REVISION
from ollama_provider import proposal_targets

LIMIT = 8 * 1024 * 1024


def verify_model(root):
    provenance = json.loads((root / 'provenance.json').read_text())
    if provenance['repository'] != REPOSITORY or provenance['revision'] != REVISION:
        raise ValueError('Expected the pinned T5-base model revision')
    for name, expected in EXPECTED.items():
        if provenance['files'][name]['sha256'] != expected or digest(root / name) != expected:
            raise ValueError(f'Model artifact hash mismatch: {name}')
    return dict(repository=REPOSITORY, revision=REVISION, files=EXPECTED,
                provenance_sha256=digest(root / 'provenance.json'))


def prepare(request, tokenizer, context_tokens=64):
    if request.get('schema_version') != 1 or request.get('task') != 'propose_context_candidates':
        raise ValueError('Expected context-proposal protocol version 1')
    limit = request['limits']['max_proposals']
    byte_limit = request['limits']['max_text_bytes']
    if (type(limit) is not int or not 1 <= limit <= 4096
            or type(byte_limit) is not int or not 1 <= byte_limit <= 512):
        raise ValueError('Invalid proposal limits')
    targets = proposal_targets(request)
    if limit < len(targets):
        raise ValueError('Proposal budget must allow at least one per context')
    first = tokenizer.convert_tokens_to_ids('<extra_id_0>')
    end = tokenizer.convert_tokens_to_ids('<extra_id_1>')
    if first == end or first not in tokenizer.all_special_ids or end not in tokenizer.all_special_ids:
        raise ValueError('Tokenizer lacks distinct span sentinels')
    prepared = {}
    reserved = set(tokenizer.all_special_ids) - {tokenizer.unk_token_id}
    for target in targets:
        left, right = target['left_context'], target['right_context']
        for side in (left, right):
            if any(t in reserved for t in tokenizer.encode(side, add_special_tokens=False)):
                raise ValueError('Context contains reserved special tokens')
        # Encode the complete passage, retaining supplied within-word boundaries.
        # SentencePiece itself normalizes whitespace; record actual input IDs.
        ids = tokenizer.encode(left + '<extra_id_0>' + right, add_special_tokens=False)
        if ids.count(first) != 1:
            raise ValueError('Expected exactly one input gap')
        split = ids.index(first)
        before, after = ids[:split], ids[split + 1:]
        if not before and not after:
            raise ValueError('Empty tokenized context')
        prepared[target['id']] = dict(
            input_ids=before[-context_tokens:] + [first] + after[:context_tokens] + [tokenizer.eos_token_id],
            left_truncated=len(before) > context_tokens, right_truncated=len(after) > context_tokens)
    return prepared, limit, byte_limit


def decode_sequences(sequences, scores, tokenizer, byte_limit):
    first = tokenizer.convert_tokens_to_ids('<extra_id_0>')
    end = tokenizer.convert_tokens_to_ids('<extra_id_1>')
    if len(sequences) != len(scores):
        raise ValueError('Beam score count mismatch')
    result, seen = [], set()
    rejected = {'unterminated': 0, 'invalid_text': 0, 'duplicate': 0}
    for sequence, score in zip(sequences, scores):
        if sequence[:2] != [tokenizer.pad_token_id, first] or not math.isfinite(score) or score > 0:
            raise ValueError('Invalid decoder prefix or score')
        span = sequence[2:]
        if end not in span:
            rejected['unterminated'] += 1
            continue
        stop = span.index(end)
        # Transformers beam finalization can fill finished rows with repeated
        # EOS IDs (our closing sentinel), as well as the configured padding ID.
        if any(t not in (tokenizer.pad_token_id, end) for t in span[stop + 1:]):
            raise ValueError('Unexpected tokens after closing sentinel')
        tokens = span[:stop]
        text = tokenizer.decode(tokens, skip_special_tokens=False, clean_up_tokenization_spaces=False).strip()
        if (not text or len(text.encode()) > byte_limit
                or any(unicodedata.category(c) == 'Cc' for c in text)
                or any(t in tokenizer.all_special_ids for t in tokens)):
            rejected['invalid_text'] += 1
            continue
        if text in seen:
            rejected['duplicate'] += 1
            continue
        seen.add(text)
        result.append(dict(text=text, tokens=tokens, beam_log_score=score))
    return result, rejected


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
    prepared, limit, byte_limit = prepare(request, tokenizer, args.context_tokens)
    first = tokenizer.convert_tokens_to_ids('<extra_id_0>')
    end = tokenizer.convert_tokens_to_ids('<extra_id_1>')
    options = dict(num_beams=args.beams, num_return_sequences=args.beams,
                   max_new_tokens=args.tokens, length_penalty=0.0,
                   early_stopping=True, do_sample=False)
    groups, diagnostics = [], {}
    with torch.inference_mode():
        for target, entry in prepared.items():
            output = model.generate(torch.tensor([entry['input_ids']]),
                decoder_input_ids=torch.tensor([[tokenizer.pad_token_id, first]]),
                eos_token_id=end, pad_token_id=tokenizer.pad_token_id,
                suppress_tokens=[t for t in tokenizer.all_special_ids if t != end],
                return_dict_in_generate=True, output_scores=True, **options)
            sequences, scores = output.sequences.tolist(), output.sequences_scores.tolist()
            candidates, rejected = decode_sequences(sequences, scores, tokenizer, byte_limit)
            diagnostics[target] = dict(candidates=candidates, rejected=rejected,
                                       sequences=sequences, sequence_scores=scores)
            groups.append([dict(target=target, text=c['text']) for c in candidates])
    # Round-robin prevents early occurrences from consuming the shared budget.
    proposals = [p for row in itertools.zip_longest(*groups) for p in row if p is not None][:limit]
    files = ['span_proposal_provider.py', 'ollama_provider.py', 'fetch_span_model.py', 'fetch_context_model.py']
    result = dict(schema_version=1, proposals=proposals, provenance=dict(
        provider='offline-t5-span', model=provenance, options=options,
        context_tokens=args.context_tokens, threads=args.threads,
        torch_version=torch.__version__, transformers_version=transformers.__version__,
        request_sha256=hashlib.sha256(json.dumps(request, sort_keys=True).encode()).hexdigest(),
        source_hashes={name: digest(Path(__file__).with_name(name)) for name in files},
        contexts_used=prepared, diagnostics=diagnostics,
        interpretation='Bounded span-denoising beam search, conditioned on an opening sentinel and stopped at its closing sentinel. Scores are unnormalized search diagnostics, not recovery confidence. Independent context proposals are combined by the Rust physical solver.'))
    if adapter is not None:
        result['provenance']['domain_adapter'] = adapter
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model-dir', type=Path, required=True)
    parser.add_argument('--adapter', type=Path, help='Optional verified local domain adapter directory')
    parser.add_argument('--beams', type=int, default=32)
    parser.add_argument('--tokens', type=int, default=24, help='Decoder budget including closing sentinel')
    parser.add_argument('--context-tokens', type=int, default=64, help='Nearest visible tokens retained per side')
    parser.add_argument('--threads', type=int, default=4)
    args = parser.parse_args()
    if not (2 <= args.beams <= 64 and 2 <= args.tokens <= 64
            and 1 <= args.context_tokens <= 192 and 1 <= args.threads <= 32):
        parser.error('Invalid inference limits')
    raw = sys.stdin.buffer.read(LIMIT + 1)
    if len(raw) > LIMIT:
        parser.error('Request exceeds 8 MB')
    try:
        result = run(json.loads(raw), args)
        output = json.dumps(result, ensure_ascii=False)
        if len(output.encode()) > LIMIT:
            raise ValueError('Response exceeds 8 MB')
    except (ValueError, KeyError, TypeError, OSError) as error:
        parser.exit(1, f'Span proposal provider: {error}\n')
    print(output)


if __name__ == '__main__':
    main()
