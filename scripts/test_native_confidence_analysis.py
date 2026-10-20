# Copyright (C) 2026 Toit contributors.
import copy
import unittest
from analyze_native_confidence_controls import records


class NativeConfidenceAnalysisTests(unittest.TestCase):
    def fixture(self):
        language=[];native=[]
        for doc in ['a','b']:
            for availability in ['present','absent']:
                base=dict(case=doc+'-'+availability,document=doc,availability=availability)
                language.append(dict(**base,tracks=dict(primary=dict(status='error' if doc=='b' else 'ok',unique_correct_first=True))))
                native.append(dict(**base,status='failed_or_unsupported' if doc=='b' else 'ok',zero_replay_exact=True,
                    variants={} if doc=='b' else dict(neutral=dict(selected_correct=True),prior=dict(selected_correct=False))))
        return dict(rows=language),dict(rows=native)

    def analyze(self,language,native):return records(language,native,'primary',['neutral','prior'],dict(a='x',b='y'),2)

    def test_failures_are_included_without_inventing_predictions(self):
        language,native=self.fixture();rows=self.analyze(language,native)
        self.assertEqual(len(rows),6)
        self.assertEqual(sum(r['correct_first'] for r in rows),2)
        self.assertTrue(all(not r['correct_first'] for r in rows if r['document']=='b'))

    def test_mismatched_or_incomplete_paired_inputs_are_rejected(self):
        language,native=self.fixture()
        changes=[lambda j:j['rows'].pop(),lambda j:j['rows'].append(copy.deepcopy(j['rows'][0])),
            lambda j:j['rows'][0].update(document='b'),lambda j:j['rows'][0].update(zero_replay_exact=False),
            lambda j:j['rows'][0]['variants'].pop('prior')]
        for change in changes:
            invalid=copy.deepcopy(native);change(invalid)
            with self.assertRaises(ValueError):self.analyze(language,invalid)


if __name__=='__main__':unittest.main()
