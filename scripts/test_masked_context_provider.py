# Copyright (C) 2026 Toit contributors.
import unittest
from masked_context_provider import prepare, states_at_step


class Tokenizer:
    all_special_ids = [0]

    def encode(self, text, add_special_tokens=False):
        return [0 if word == '[MASK]' else int(word) for word in text.split()]


class ContextTests(unittest.TestCase):
    def request(self):
        return dict(schema_version=1, task='score_context_candidates',
            contexts={'t': dict(left='1 2 3', right='4 5 6')},
            items=[dict(id='a', target='t', text='7 8'),
                   dict(id='b', target='t', text='7 9'),
                   dict(id='c', target='t', text='10')])

    def test_near_context_and_explicit_truncation(self):
        items, contexts = prepare(self.request(), Tokenizer(), 2)
        self.assertEqual(items[0]['left'], (2, 3))
        self.assertEqual(items[0]['right'], (4, 5))
        self.assertTrue(contexts['t']['left_truncated'])
        self.assertTrue(contexts['t']['right_truncated'])

    def test_candidates_do_not_leak_future_wordpieces(self):
        items, _ = prepare(self.request(), Tokenizer())
        first = states_at_step(items, 0)
        self.assertEqual(len(first), 2)  # One shared state per candidate length.
        self.assertTrue(all(state[3] == () for state, _ in first))
        second = states_at_step(items, 1)
        self.assertEqual(len(second), 1)
        self.assertEqual(second[0][0][3], (7,))
        self.assertEqual(second[0][1], [(0, 8), (1, 9)])
        reversed_items = list(reversed(items))
        self.assertEqual([s for s, _ in first], [s for s, _ in states_at_step(reversed_items, 0)])

    def test_invalid_tokens_and_duplicate_items_fail(self):
        for modification in ['unknown', 'duplicate', 'special', 'empty']:
            r = self.request()
            if modification == 'unknown': r['items'][0]['target'] = 'missing'
            if modification == 'duplicate': r['items'].append(r['items'][0])
            if modification == 'special': r['items'][0]['text'] = '[MASK]'
            if modification == 'empty': r['contexts']['t'] = dict(left='', right='')
            with self.assertRaises(ValueError): prepare(r, Tokenizer())


if __name__ == '__main__':
    unittest.main()
