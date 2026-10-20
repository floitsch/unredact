# Copyright (C) 2026 Toit contributors.
import unittest
from analyze_heldout_families import family_analysis


class FamilyTests(unittest.TestCase):
    def records(self):
        rows=[]
        for doc in ['a1','a2','a3','b1']:
            for index in range(2):
                for track in ['primary','control','unexposed']:
                    rows.append(dict(case=f'{doc}-{index}',document=doc,condition='opaque',track=track,
                        status='not_available' if track=='unexposed' else ('error' if doc=='b1' else 'ok'),
                        correct_first=track=='primary' and doc!='b1'))
        return rows

    def analyze(self,rows):
        return family_analysis(rows,families={'a1':'family-a','a2':'family-a','a3':'family-a','b1':'family-b'},
            tracks=['primary','control','unexposed'],conditions=['opaque'],primary='primary',cases_per_document=2)

    def test_family_weights_failures_and_unavailable_interfaces(self):
        result=self.analyze(self.records());primary=next(m for m in result['metrics'] if m['track']=='primary')
        self.assertEqual(primary['cases'],8);self.assertEqual(primary['family_macro_first'],.5)
        self.assertEqual(primary['family_cases'],{'family-a':6,'family-b':2})
        missing=next(m for m in result['metrics'] if m['track']=='unexposed')
        self.assertIsNone(missing['family_macro_first']);self.assertIsNone(missing['family_95_interval'])
        self.assertEqual(next(p for p in result['paired'] if p['second']=='unexposed')['status'],'not_available')
        self.assertEqual(result,self.analyze(self.records()[::-1]))

    def test_incomplete_or_duplicate_cases_are_rejected(self):
        with self.assertRaises(ValueError):self.analyze(self.records()[:-1])
        with self.assertRaises(ValueError):self.analyze(self.records()+[self.records()[0]])
        with self.assertRaises(ValueError):self.analyze([r for r in self.records() if r['track']!='control'])


if __name__=='__main__':unittest.main()
