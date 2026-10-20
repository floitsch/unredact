# Copyright (C) 2026 Toit contributors.
import copy
import importlib.util
import unittest

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
