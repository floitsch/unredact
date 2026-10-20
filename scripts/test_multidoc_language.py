# Copyright (C) 2026 Toit contributors.
import unittest

from evaluate_multidoc_language import grade, summarize, verify_ranked, TRACKS


class CommonLanguageTests(unittest.TestCase):
    def test_zero_weight_cli_omits_combined_field(self):
        scores, costs = {'a': -2.}, {'a': 1.5}
        verify_ranked([dict(id='a', mean_sequence_log_score=-2.)], {'a'}, scores, costs, 0)
        verify_ranked([dict(id='a', mean_sequence_log_score=-2., combined_score=-3.5)], {'a'}, scores, costs, 1)
        with self.assertRaises(ValueError):
            verify_ranked([dict(id='a', mean_sequence_log_score=-2., combined_score=-2.)], {'a'}, scores, costs, 1)

    def test_ties_filters_and_absence(self):
        scores = {'truth': -2., 'other': -2., 'bad': -5.}
        tied = grade(scores, scores, 'truth')
        self.assertEqual(tied['truth_rank'], [1, 2])
        self.assertFalse(tied['unique_correct_first'])
        self.assertEqual(grade(scores, {'truth', 'bad'}, 'truth')['truth_rank'], [1, 1])
        self.assertIsNone(grade(scores, {'other'}, 'truth')['truth_rank'])
        self.assertEqual(grade(scores, None, 'truth')['status'], 'not_available')
        absent = grade(scores, set(), None)
        self.assertTrue(absent['empty_set_proxy'])
        self.assertIsNone(absent['truth_compatible'])
        self.assertFalse(absent['unique_correct_first'])
        self.assertTrue(grade(scores, {'bad'}, None)['single_wrong_candidate'])
        combined = grade(scores, scores, 'truth', {'truth': 4., 'other': 0., 'bad': 0.}, 1)
        self.assertEqual(combined['truth_rank'], [3, 3])
        with self.assertRaises(ValueError):
            grade(scores, {'unknown'}, 'truth')

    def test_document_weights_failure_denominators_and_unavailable_api(self):
        rows = []
        for doc, count, success in [('a', 9, True), ('b', 1, False)]:
            for _ in range(count):
                values = {t: grade({'truth': -1.}, {'truth'}, 'truth') if success else
                          dict(status='error', reason='failed') for t in TRACKS}
                values[TRACKS[-1]] = dict(status='not_available')
                rows.append(dict(availability='present', document=doc, source_group=doc,
                                 kind='scan', word_count=1, tracks=values))
        result = summarize(rows)
        all_rows = [r for r in result['metrics'] if r['breakdown'] == 'all']
        local = next(r for r in all_rows if r['track'] == TRACKS[0])
        self.assertEqual(local['cases'], 10)
        self.assertEqual(local['unique_correct_first'], 9)
        self.assertEqual(local['document_macro_first'], .5)
        missing = next(r for r in all_rows if r['track'] == TRACKS[-1])
        self.assertIsNone(missing['unique_correct_first'])
        self.assertFalse(any(TRACKS[-1] in (p['first'], p['second']) for p in result['paired']))
        self.assertEqual(result, summarize(rows[::-1]))


if __name__ == '__main__':
    unittest.main()
