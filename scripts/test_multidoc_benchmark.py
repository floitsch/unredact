# Copyright (C) 2026 Toit contributors.
import unittest

import numpy as np
from PIL import Image

from prepare_multidoc_benchmark import damage, pixelate, select_spans, target_evidence
from summarize_multidoc_benchmark import cluster_interval, summarize


class BenchmarkTests(unittest.TestCase):
    def test_generated_render_positions_stay_inside_rounded_crop(self):
        for box in [(100,200,180,228),(100.375,200.25,180.375,228.25)]:
            evidence=target_evidence('pixelate-8',box)
            self.assertAlmostEqual(evidence['origin_x'],box[0]%1)
            self.assertAlmostEqual(evidence['baseline_y'],26+box[1]%1)
            self.assertEqual(evidence,target_evidence('pixelate-16',box))
        for mode in ['opaque','opaque-low-resolution','opaque-jpeg-noise','opaque-subpixel']:
            self.assertEqual(target_evidence(mode,(100,200,180,228)),dict(kind='opaque',padding_min=0.,padding_max=12.,measurement_sigma=1.5))

    def test_opaque_damage_cannot_depend_on_hidden_pixels(self):
        # Identical visible content, different hidden text: all opaque variants
        # must be exactly identical, including antialiasing/compression fringes.
        a=Image.new('L',(180,100),255);b=a.copy()
        b.paste(0,(40,30,110,65));boxes=[(35,25,115,70)]
        for mode in ['opaque','opaque-low-resolution','opaque-jpeg-noise','opaque-subpixel']:
            x,xb=damage(a,boxes,mode,123);y,yb=damage(b,boxes,mode,123)
            self.assertEqual(x.tobytes(),y.tobytes(),mode);self.assertEqual(xb,yb)
        self.assertNotEqual(pixelate(a,boxes,8).tobytes(),pixelate(b,boxes,8).tobytes())

    def test_pixelation_matches_block_mean_and_preserves_context(self):
        a=np.arange(16*16,dtype=np.uint8).reshape(16,16)
        b=np.asarray(pixelate(Image.fromarray(a),[(3,5,13,14)],8))
        self.assertEqual(b[5,3],round(float(a[:8,:8].mean())))
        self.assertEqual(b[12,12],round(float(a[8:16,8:16].mean())))
        np.testing.assert_array_equal(b[:5],a[:5]);np.testing.assert_array_equal(b[:,13:],a[:,13:])

    def test_failures_missing_and_paired_document_statistics(self):
        cases={f'{g}-{i}':dict(split='test',kind='scan',condition='opaque',availability='present',source_group=g)
               for g in ['a','b','c'] for i in range(4)}
        rows=[dict(adapter=a,case=c,repetition=0,result=dict(status='error' if c.startswith('b') else 'ok'),
                   metrics=dict(unique_correct_first=c.startswith('a') and a=='local'))
              for a in ['local','external'] for c in cases]
        summary=summarize(cases,rows,['local','external'])[0]
        self.assertAlmostEqual(summary['configurations']['local']['document_macro_accuracy'],1/3)
        self.assertEqual(summary['configurations']['local']['failed_executions'],4)
        self.assertAlmostEqual(summary['paired'][0]['macro_difference'],1/3)
        # Repeating every trial must not narrow the cluster interval.
        repeated=rows+[dict(r,repetition=1) for r in rows]
        second=summarize(cases,repeated,['local','external'])[0]
        self.assertEqual(summary['paired'],second['paired'])
        incomplete=summarize(cases,rows[1:],['local','external'])[0]['configurations']['local']
        self.assertFalse(incomplete['complete']);self.assertIsNone(incomplete['document_macro_accuracy'])
        with self.assertRaises(ValueError):summarize(cases,rows+rows,['local','external'])
        self.assertIsNone(cluster_interval([1.]))


if __name__=='__main__':unittest.main()
