# Copyright (C) 2026 Toit contributors.
import unittest

from analyze_multidoc_geometry import aggregate


def row(group,status='ok',exposed=True,correct=True,present=True):
    metrics=dict(unique_correct_first=correct,truth_in_best_group=True,truth_rank=[1,8],
                 compatible_candidates=8 if exposed else None,truth_compatible=True if exposed else None,
                 no_compatible=False if exposed else None,single_wrong_compatible=False if exposed else None) if status=='ok' else {}
    return dict(source_group=group,status=status,availability='present' if present else 'absent',metrics=metrics,elapsed_seconds=1.)


class GeometryReportTests(unittest.TestCase):
    def test_unreported_compatibility_is_unknown(self):
        result=aggregate([row('a',exposed=False),row('b',status='error')])
        self.assertIsNone(result['truth_compatible'])
        self.assertEqual(result['compatibility_unreported_successes'],1)
        result=aggregate([row('a',exposed=False,present=False)])
        self.assertIsNone(result['empty_compatible_set'])
        self.assertIsNone(result['single_wrong_compatible'])

    def test_failures_ties_and_equal_document_weights(self):
        rows=[row('a')]*9+[row('b',status='unsupported')]
        result=aggregate(rows)
        self.assertEqual(result['unique_correct_first'],9)
        self.assertEqual(result['truth_top5_guaranteed'],0)
        self.assertEqual(result['truth_top5_possible'],9)
        self.assertEqual(result['document_macro_first'],.5)
        self.assertEqual(result['truth_compatible'],9)
        self.assertEqual(result['document_95_interval'],aggregate(rows[::-1])['document_95_interval'])


if __name__=='__main__':unittest.main()
