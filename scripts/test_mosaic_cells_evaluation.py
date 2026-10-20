# Copyright (C) 2026 Toit contributors.
import copy
from pathlib import Path
import tempfile
import unittest

from PIL import Image
from evaluate_mosaic_cells import digest, prepare_request, summarize, write


class MosaicEvaluationTests(unittest.TestCase):
    def test_cached_typography_requires_identical_visible_pixels(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            project = root / 'work/project'
            project.mkdir(parents=True)
            bounds = dict(x=1., y=1., w=2., h=2.)
            image = Image.new('L', (4, 4), 240)
            image.save(project / 'page.pgm')
            altered = image.copy()
            altered.putpixel((1, 1), 10)
            altered.save(root / 'mosaic.pgm')
            data = dict(targets=[dict(page='p', profile='f', bounds=bounds)],
                        pages=[dict(id='p', image='page.pgm')],
                        profiles=[dict(id='f', training_texts=['visible'], validation_texts=[], fits=[])])
            write(project / 'project.json', data)
            case = dict(input=dict(targets=[dict(page='p', bounds=bounds, evidence=dict(kind='mosaic_auto'))],
                                   pages=[dict(id='p', image='mosaic.pgm', image_sha256=digest(root / 'mosaic.pgm'), redaction_regions=[bounds])],
                                   candidates=[]))
            before = copy.deepcopy(case)
            request, legacy, _ = prepare_request(root, case, root / 'work')
            self.assertEqual(request['target_index'], 0)
            self.assertEqual(request['profile']['training_texts'], [])
            self.assertEqual(legacy['targets'][0]['evidence'], dict(kind='mosaic_auto'))
            self.assertEqual(case, before)
            altered.putpixel((0, 0), 10)
            altered.save(root / 'mosaic.pgm')
            case['input']['pages'][0]['image_sha256'] = digest(root / 'mosaic.pgm')
            with self.assertRaisesRegex(ValueError, 'Visible pixels changed'):
                prepare_request(root, case, root / 'work')

    def test_failures_and_absent_controls_do_not_become_successes(self):
        rows = []
        for document, count, success in [('a', 9, True), ('b', 1, False)]:
            for _ in range(count):
                rows.append(dict(document=document, condition='pixelate-8', availability='present', kind='scan',
                    engines=dict(legacy=dict(status='error'), shared_cells=dict(status='ok', truth_rank=[1, 1]) if success else dict(status='timeout'))))
        result = summarize(rows)
        metric = next(r for r in result['metrics'] if r['breakdown'] == 'all' and r['engine'] == 'shared_cells')
        self.assertEqual(metric['cases'], 10)
        self.assertEqual(metric['unique_correct_first'], 9)
        self.assertEqual(metric['document_macro_first'], .5)
        rows[0]['availability'] = 'absent'
        result = summarize(rows[:1])
        self.assertTrue(all(r['absence_accuracy'] is None for r in result['metrics']))


if __name__ == '__main__':
    unittest.main()
