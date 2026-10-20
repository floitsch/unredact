# Copyright (C) 2026 Toit contributors.
import unittest
from masked_proposal_provider import balanced, generate, prepare


class Tokenizer:
    vocab = {'[UNK]': 0, '[MASK]': 1, 'cat': 2, 'dog': 3, '##s': 4, 'black': 5, 'dogs': 6}
    all_special_ids = [0, 1]
    unk_token_id = 0

    def get_vocab(self):
        return self.vocab

    def encode(self, text, add_special_tokens=False):
        out = []
        for word in text.split():
            if word in self.vocab:
                out.append(self.vocab[word])
            elif word.startswith('cat') and set(word[3:]) == {'s'}:
                out.extend([2] + [4] * len(word[3:]))
            else:
                out.append(0)
        return out

    def decode(self, tokens, **kwargs):
        reverse = {v: k for k, v in self.vocab.items()}
        return ' '.join(reverse[t] for t in tokens).replace(' ##', '')


class ProposalTests(unittest.TestCase):
    def test_multi_piece_and_phrase_generation_without_future_text(self):
        visited = []

        def predict(states, allowed, width):
            visited.extend(states)
            return [[(t, -float(t)) for t in ids][:width] for ids in allowed]

        result, _ = generate((5,), (3,), Tokenizer(), predict, 3, 2, 16, 512)
        texts = {c['text'] for c in result}
        self.assertIn('cats', texts)
        self.assertIn('black cat', texts)
        self.assertIn('black cats', texts)
        self.assertTrue(all(len(c['tokens']) == len(Tokenizer().encode(c['text'])) for c in result))
        self.assertNotIn([3, 4], [c['tokens'] for c in result])  # dogs has its own token
        self.assertTrue(all(len(prefix) < length for _, _, length, prefix in visited))
        self.assertTrue(all(left == (5,) and right == (3,) for left, right, _, _ in visited))
        result, _ = generate((), (3,), Tokenizer(), predict, 3, 1, 16, 512)
        self.assertTrue(all(' ' not in c['text'] for c in result))

    def test_balancing_does_not_starve_later_groups(self):
        self.assertEqual(list(balanced([[1, 2, 3], [4], [5, 6]])), [1, 4, 5, 2, 6, 3])

    def test_context_limits_special_tokens_and_canonical_order(self):
        request = dict(schema_version=1, task='propose_context_candidates',
                       contexts={'z': dict(left='cat dog black', right='dog'),
                                 'a': dict(left='black', right='cat')},
                       limits=dict(max_proposals=8, max_text_bytes=512))
        encoded, used, _, _ = prepare(request, Tokenizer(), 2)
        self.assertEqual(list(encoded), ['a', 'z'])
        self.assertEqual(encoded['z'][0], (3, 5))
        self.assertTrue(used['z']['left_truncated'])
        for bad in ['', '[MASK]', 'x' * 4097]:
            request['contexts'] = {'t': dict(left=bad, right='')}
            with self.assertRaises(ValueError):
                prepare(request, Tokenizer())

    def test_predicted_out_of_scope_or_invalid_scores_are_rejected(self):
        for option in [(1, -1.), (2, float('nan')), (2, 1.)]:
            with self.assertRaises(ValueError):
                generate((), (3,), Tokenizer(), lambda states, allowed, width: [[option]], 1, 1, 2, 512)


if __name__ == '__main__':
    unittest.main()
