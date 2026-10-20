# Copyright (C) 2026 Toit contributors.
import copy
import unittest
from prepare_scan_holdout import matching_spans


class SpanTests(unittest.TestCase):
    def test_phrase_geometry_and_ocr_line_boundaries(self):
        o = dict(text=['', 'manual', 'control', '', 'manual', 'control'],
                 left=[0, 10, 80, 0, 10, 80], top=[0, 10, 10, 0, 40, 40],
                 width=[0, 60, 65, 0, 60, 65], height=[0, 20, 20, 0, 20, 20],
                 block_num=[0, 1, 1, 0, 1, 1], par_num=[0, 1, 1, 0, 1, 1],
                 line_num=[0, 1, 1, 0, 2, 2])
        self.assertEqual(matching_spans(o, 'manual control'), [[1, 2], [4, 5]])
        self.assertEqual(matching_spans(o, 'control'), [[2], [5]])
        self.assertEqual(matching_spans(o, 'control manual'), [])
        spaced = copy.deepcopy(o)
        spaced['text'][1] = ' manual'
        self.assertEqual(matching_spans(spaced, 'manual control'), [[1, 2], [4, 5]])
        for key, value in [('left', 500), ('left', 20), ('top', 100), ('block_num', 2), ('par_num', 2), ('line_num', 2)]:
            changed = copy.deepcopy(o)
            changed[key][2] = value
            self.assertEqual(matching_spans(changed, 'manual control'), [[4, 5]], (key, value))


if __name__ == '__main__':
    unittest.main()
