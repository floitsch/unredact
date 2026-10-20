# Copyright (C) 2026 Toit contributors.
import copy
import unittest
from correct_mosaic_coordinates import correct


class CoordinateAmendmentTests(unittest.TestCase):
    def test_mask_only_conversion_and_no_double_application(self):
        target = dict(bounds=dict(x=100.375, y=200.25, w=80., h=28.),
                      evidence=dict(kind='mosaic_auto', origin_x=100.375, baseline_y=226.25,
                                    min_block=4, max_block=20, noise_sigma=8.))
        before = copy.deepcopy(target)
        fixed = correct(target)
        self.assertEqual(fixed['evidence']['origin_x'], .375)
        self.assertEqual(fixed['evidence']['baseline_y'], 26.25)
        self.assertEqual(target, before)
        self.assertEqual(fixed['bounds'], target['bounds'])
        with self.assertRaises(ValueError):
            correct(fixed)
        opaque = dict(bounds=target['bounds'], evidence=dict(kind='opaque', padding_max=12.))
        self.assertEqual(correct(opaque), opaque)


if __name__ == '__main__':
    unittest.main()
