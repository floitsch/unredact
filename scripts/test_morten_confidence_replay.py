# Copyright (C) 2026 Toit contributors.
import math
import unittest
from replay_morten_confidence import relative_weights


class RelativeWeightTests(unittest.TestCase):
    def test_extreme_log_scores_and_offset_invariance(self):
        weights=relative_weights({'a':-10000.,'b':-10001.,'c':-20000.},{'a','b','c'})
        self.assertAlmostEqual(sum(weights.values()),1.)
        self.assertGreater(weights['a'],weights['b']);self.assertEqual(weights['c'],0.)
        shifted=relative_weights({'a':0.,'b':-1.,'c':-10000.},{'a','b','c'})
        self.assertEqual(weights,shifted)
    def test_missing_or_nonfinite_scores_rejected(self):
        with self.assertRaises(ValueError):relative_weights({'a':-1.},{'a','b'})
        with self.assertRaises(ValueError):relative_weights({'a':math.nan},{'a'})


if __name__=='__main__':unittest.main()
