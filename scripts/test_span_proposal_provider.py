# Copyright (C) 2026 Toit contributors.
import copy
import re
import unittest

from span_proposal_provider import decode_sequences, prepare


class Tokenizer:
    vocab = {'<pad>': 0, '</s>': 1, '<unk>': 2, '<extra_id_0>': 3,
             '<extra_id_1>': 4, 'left': 5, 'right': 6, 'partial': 7,
             'control': 8, 'more': 9, 'bad': 10}
    all_special_ids = [0, 1, 2, 3, 4]
    pad_token_id, eos_token_id, unk_token_id = 0, 1, 2

    def convert_tokens_to_ids(self, token):
        return self.vocab.get(token, 2)

    def encode(self, text, **kwargs):
        return [self.convert_tokens_to_ids(t) for t in re.findall(r'<[^>]+>|\w+', text)]

    def decode(self, tokens, **kwargs):
        reverse = {v: k for k, v in self.vocab.items()}
        return ' '.join(reverse[t] for t in tokens).replace('bad', 'bad\ntext')


class SpanTests(unittest.TestCase):
    def setUp(self):
        self.request = dict(schema_version=1, task='propose_context_candidates',
            surface_kind='canonical', limits=dict(max_proposals=8, max_text_bytes=512),
            contexts={'z': dict(left='more left ', right=' right more'),
                      'a': dict(left='left ', right=' right')})

    def test_gap_preserves_nearest_context_and_records_truncation(self):
        prepared, limit, byte_limit = prepare(self.request, Tokenizer(), 1)
        self.assertEqual(list(prepared), ['a', 'z'])
        self.assertEqual(prepared['z'], dict(input_ids=[5, 3, 6, 1], left_truncated=True, right_truncated=True))
        self.assertFalse(prepared['a']['left_truncated'])
        self.assertEqual((limit, byte_limit), (8, 512))
        # A candidate list accidentally attached by a caller must not condition generation.
        extra = copy.deepcopy(self.request)
        extra['candidates'] = [{'text': 'partial control'}]
        self.assertEqual(prepare(extra, Tokenizer(), 1)[0], prepared)

    def test_untrusted_context_cannot_add_decoder_sentinels(self):
        for bad in ['<extra_id_0>', '<extra_id_1>', '</s>', '<pad>', 'x' * 4097]:
            request = copy.deepcopy(self.request)
            request['contexts']['a']['left'] = bad
            with self.assertRaises(ValueError):
                prepare(request, Tokenizer())
        self.request['limits']['max_proposals'] = 1
        with self.assertRaisesRegex(ValueError, 'per context'):
            prepare(self.request, Tokenizer())

    def test_only_completed_nonempty_spans_are_proposed(self):
        rows = [[0, 3, 7, 8, 4, 4, 0], [0, 3, 7, 8], [0, 3, 4, 0],
                [0, 3, 7, 8, 4], [0, 3, 10, 4], [0, 3, 2, 4]]
        candidates, rejected = decode_sequences(rows, [-1.] * len(rows), Tokenizer(), 512)
        self.assertEqual([c['text'] for c in candidates], ['partial control'])
        self.assertEqual(rejected, dict(unterminated=1, invalid_text=3, duplicate=1))
        short, _ = decode_sequences(rows[:1], [-1.], Tokenizer(), 4)
        self.assertEqual(short, [])

    def test_decoder_corruption_and_bad_scores_are_errors(self):
        for sequence, score in [([0, 7, 8, 4], -1.), ([0, 3, 7, 4, 8], -1.),
                                ([0, 3, 7, 4], float('nan')), ([0, 3, 7, 4], .1)]:
            with self.assertRaises(ValueError):
                decode_sequences([sequence], [score], Tokenizer(), 512)


if __name__ == '__main__':
    unittest.main()
