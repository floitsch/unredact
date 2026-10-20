# Copyright (C) 2026 Toit contributors.
import copy
import importlib.util
import io
import json
import types
import unittest
from unittest.mock import Mock, patch

import span_context_provider as provider
from span_context_provider import decoder_sequence, prepare, score_items
from test_span_proposal_provider import Tokenizer


class SpanContextTests(unittest.TestCase):
    def request(self):
        return dict(schema_version=1, task='score_context_candidates',
            contexts={'t': dict(left='more left ', right=' right more')},
            items=[dict(id='short', target='t', text='control'),
                   dict(id='long', target='t', text='partial control')])

    def test_candidate_length_and_text_do_not_enter_encoder(self):
        items, contexts = prepare(self.request(), Tokenizer(), 1)
        self.assertEqual(contexts['t']['input_ids'], [5, 3, 6, 1])
        self.assertTrue(contexts['t']['left_truncated'])
        self.assertEqual(items[0]['tokens'], [8])
        self.assertEqual(items[1]['tokens'], [7, 8])
        changed = self.request()
        changed['items'] = [dict(id='other', target='t', text='more')]
        self.assertEqual(prepare(changed, Tokenizer(), 1)[1], contexts)
        self.assertEqual(decoder_sequence([7, 8], Tokenizer()), ([0, 3, 7, 8], [7, 8, 4]))

    def test_invalid_scores_cannot_silently_shorten_candidate(self):
        for modification in ['target', 'duplicate', 'special', 'empty', 'long', 'control']:
            request = self.request()
            if modification == 'target': request['items'][0]['target'] = 'unknown'
            if modification == 'duplicate': request['items'].append(request['items'][0])
            if modification == 'special': request['items'][0]['text'] = '<extra_id_1>'
            if modification == 'empty': request['items'][0]['text'] = ' '
            if modification == 'long': request['items'][0]['text'] = 'left ' * 65
            if modification == 'control': request['items'][0]['text'] = 'partial\ncontrol'
            with self.assertRaises(ValueError):
                prepare(request, Tokenizer())

    def test_worker_loads_once_preserves_response_provenance_and_flushes_each_frame(self):
        requests = [self.request(), self.request()]
        requests[1]['items'] = requests[1]['items'][:1]
        raw = b''.join(json.dumps(dict(worker_protocol_version=1, request_id=i + 1,
            request=request)).encode() + b'\n' for i, request in enumerate(requests))
        provenance = dict(model='verified-test-model')
        scorer = Mock()
        scorer.score.side_effect = lambda request: dict(provenance=provenance, items=request['items'])
        factory = Mock(return_value=scorer)
        output = io.BytesIO()
        provider.serve(io.BytesIO(raw), output, 'options', factory)
        factory.assert_called_once_with('options')
        self.assertEqual(scorer.score.call_count, 2)
        frames = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual([frame['request_id'] for frame in frames], [1, 2])
        for frame, request in zip(frames, requests):
            self.assertEqual(frame['response'], dict(provenance=provenance, items=request['items']))

    def test_worker_rejects_unbounded_unframed_or_repeated_request_ids(self):
        factory = Mock()
        malformed = [b'{}', b'{}\n', b'x' * 102 + b'\n',
            json.dumps(dict(worker_protocol_version=1, request_id=True, request={})).encode() + b'\n']
        with patch.object(provider, 'LIMIT', 100):
            for raw in malformed:
                with self.assertRaises(ValueError):
                    provider.serve(io.BytesIO(raw), io.BytesIO(), None, factory)
        factory.assert_not_called()
        frame = json.dumps(dict(worker_protocol_version=1, request_id=1, request={})).encode() + b'\n'
        factory.return_value.score.return_value = {}
        with self.assertRaisesRegex(ValueError, 'envelope'):
            provider.serve(io.BytesIO(frame * 2), io.BytesIO(), None, factory)
        with patch.object(provider, 'LIMIT', 10):
            with self.assertRaisesRegex(ValueError, 'Response exceeds'):
                provider.encode_response({'response': 'too large'})

    def test_model_verification_and_loading_are_amortized_across_requests(self):
        torch = types.SimpleNamespace(__version__='test-torch', set_num_threads=Mock())
        tokenizer_loader, model_loader = Mock(), Mock()
        tokenizer_loader.from_pretrained.return_value = Tokenizer()
        model = model_loader.from_pretrained.return_value.eval.return_value
        transformers = types.SimpleNamespace(__version__='test-transformers',
            T5TokenizerFast=tokenizer_loader, T5ForConditionalGeneration=model_loader)
        args = types.SimpleNamespace(model_dir='model', threads=1, context_tokens=64, batch_size=2)

        def score(items, contexts, tokenizer, loaded_model, loaded_torch, batch_size):
            self.assertIs(loaded_model, model)
            for item in items:
                item.update(token_log_score_sum=-2, token_count=2)
            return dict(unique_contexts=1)

        with patch.dict('sys.modules', torch=torch, transformers=transformers), \
                patch.object(provider, 'verify_model', return_value={'model': 'verified'}) as verify, \
                patch.object(provider, 'digest', return_value='source-sha'), \
                patch.object(provider, 'score_items', side_effect=score):
            scorer = provider.Scorer(args)
            first, second = scorer.score(self.request()), scorer.score(self.request())
        self.assertEqual(first, second)
        self.assertEqual(first['provenance'], {'model': 'verified'})
        self.assertEqual(verify.call_count, 2)  # Before and after the single load.
        tokenizer_loader.from_pretrained.assert_called_once()
        model_loader.from_pretrained.assert_called_once()

    @unittest.skipUnless(importlib.util.find_spec('torch'), 'Run with the model environment for real decoder checks')
    def test_batched_scores_equal_causal_prefix_probabilities_including_end(self):
        import torch
        from transformers import T5Config, T5ForConditionalGeneration
        torch.set_num_threads(1)
        torch.manual_seed(14)
        model = T5ForConditionalGeneration(T5Config(vocab_size=16, d_model=16, d_kv=4,
            d_ff=32, num_layers=1, num_decoder_layers=1, num_heads=4,
            dropout_rate=0, pad_token_id=0, eos_token_id=1, decoder_start_token_id=0)).eval()
        tokenizer = Tokenizer()
        contexts = {'t': dict(input_ids=[5, 3, 6, 1])}
        original = [dict(id='a', target='t', tokens=[7, 8]),
                    dict(id='b', target='t', tokens=[7, 9]),
                    dict(id='c', target='t', tokens=[7]),
                    dict(id='alias', target='t', tokens=[7, 8])]
        items = copy.deepcopy(original)
        stats = score_items(items, contexts, tokenizer, model, torch, 16)
        self.assertEqual(stats, dict(unique_contexts=1, unique_sequences=3))
        self.assertEqual(items[0]['token_log_scores'], items[3]['token_log_scores'])
        self.assertAlmostEqual(items[0]['token_log_scores'][0], items[1]['token_log_scores'][0], places=5)
        self.assertAlmostEqual(items[0]['token_log_scores'][0], items[2]['token_log_scores'][0], places=5)
        with torch.inference_mode():
            for item in items:
                prefix, expected = [0, 3], []
                for token in item['tokens'] + [4]:
                    logits = model(input_ids=torch.tensor([contexts['t']['input_ids']]),
                                   decoder_input_ids=torch.tensor([prefix]), use_cache=False).logits
                    expected.append(float(logits[0, -1].log_softmax(-1)[token]))
                    prefix.append(token)
                self.assertEqual(item['token_count'], len(item['tokens']) + 1)
                self.assertAlmostEqual(item['token_log_score_sum'], sum(expected), places=5)
        reversed_items = copy.deepcopy(original[::-1])
        score_items(reversed_items, contexts, tokenizer, model, torch, 1)
        for a, b in zip(items, reversed_items[::-1]):
            self.assertAlmostEqual(a['token_log_score_sum'], b['token_log_score_sum'], places=5)


if __name__ == '__main__':
    unittest.main()
