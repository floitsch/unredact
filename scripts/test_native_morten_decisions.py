# Copyright (C) 2026 Toit contributors.
import copy
import unittest
from audit_native_morten_decisions import decision


class NativeDecisionTests(unittest.TestCase):
    def case(self):
        return dict(truth_id='truth',input=dict(candidates=[dict(id='truth',surfaces=[dict(text='printed word')]),dict(id='other',surfaces=[dict(text='alternative')])]))
    def native(self,error=29.999):
        return dict(measurements=[dict(id='truth',text='printed word',score=error),dict(id='other',text='alternative',score=40.)],
                    matches=[dict(candidate='printed word',pct_error=error)] if error<30 else [])
    def test_strict_native_boundary_and_selection(self):
        self.assertTrue(decision(self.case(),self.native(),'present')['selected_correct'])
        rejected=decision(self.case(),self.native(30.),'present')
        self.assertTrue(rejected['rejected']);self.assertFalse(rejected['selected_correct'])
        invalid=self.native();invalid['matches']=[]
        with self.assertRaises(ValueError):decision(self.case(),invalid,'present')
    def test_coverage_and_selected_text_are_checked(self):
        for mutate in [lambda d:d['measurements'].pop(),lambda d:d['matches'][0].update(candidate='not supplied')]:
            data=self.native();mutate(data)
            with self.assertRaises(ValueError):decision(self.case(),data,'present')


if __name__=='__main__':unittest.main()
