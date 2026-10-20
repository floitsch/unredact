# Copyright (C) 2026 Toit contributors.
import copy
import unittest
from audit_language_rankings import check_report


class RankingAuditTests(unittest.TestCase):
    def report(self):
        return dict(physical=dict(independent_observations=1,candidates=[dict(id=k,evidence_score=c,compatible=True) for k,c in [('a',1.),('b',0.),('c',0.)]]),
            ranked=[dict(id=k,combined_score=s,mean_sequence_log_score=-1.) for k,s in [('b',-1.),('c',-1.),('a',-2.)]],
            ranked_groups=[['b','c'],['a']])
    def test_actual_order_and_tie_rank(self):
        value=check_report(self.report(),dict(a=-1.,b=-1.,c=-1.),1,'c')
        self.assertEqual(value['truth_rank'],[1,2]);self.assertFalse(value['unique_correct_first'])
        self.assertIsNone(check_report(self.report(),dict(a=-1.,b=-1.,c=-1.),1,None)['truth_rank'])
    def test_wrong_order_groups_or_score_fail(self):
        for change in [lambda r:r['ranked'].reverse(),lambda r:r.update(ranked_groups=[['b'],['c'],['a']]),
                       lambda r:r['ranked'][0].update(combined_score=-5.)]:
            report=copy.deepcopy(self.report());change(report)
            with self.assertRaises(ValueError):check_report(report,dict(a=-1.,b=-1.,c=-1.),1,'b')


if __name__=='__main__':unittest.main()
