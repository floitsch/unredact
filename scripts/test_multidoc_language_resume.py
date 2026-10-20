# Copyright (C) 2026 Toit contributors.
import copy
import unittest
from evaluate_multidoc_language_v2 import cached_scores


class ResumeTests(unittest.TestCase):
    def test_reuse_requires_exact_request_tokenization_and_model(self):
        request = dict(items=[dict(id='a', target='t', text='visible')], contexts={'t': 'context'})
        prepared = [dict(id='a', target='t', tokens=[12])]
        cached = dict(request=request, model={'revision': 'fixed'}, contexts_used={'t': [1]},
                      scores=[dict(**prepared[0], token_count=2, token_log_scores=[-1., -2.], token_log_score_sum=-3.)],
                      forward_states={'unique_sequences': 1})
        self.assertEqual(cached_scores(cached, request, prepared, {'t': [1]}, {'revision': 'fixed'})[0], cached['scores'])
        for mutate in [lambda d: d['scores'][0]['tokens'].append(13),
                       lambda d: d['scores'][0].update(token_log_score_sum=-4.),
                       lambda d: d['model'].update(revision='changed'),
                       lambda d: d['request']['contexts'].update(t='different')]:
            altered = copy.deepcopy(cached)
            mutate(altered)
            with self.assertRaises(ValueError):
                cached_scores(altered, request, prepared, {'t': [1]}, {'revision': 'fixed'})


if __name__ == '__main__':
    unittest.main()
