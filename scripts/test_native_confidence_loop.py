# Copyright (C) 2026 Toit contributors.
"""Integration checks using the pinned upstream selection loop itself."""
from pathlib import Path
import sys
import unittest

from replay_morten_confidence import replay

sys.path.insert(0,str(Path(__file__).resolve().parent/'adapters'))
from morten_unredactron import scoring_loop


class NativeLoopTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.loop=staticmethod(scoring_loop(Path(__file__).resolve().parent.parent/'baseline-morten/unredactron.py'))

    def case(self):return dict(input=dict(targets=[dict(bounds=dict(x=1,y=2,h=10))]))
    def native(self,width=100.):
        return dict(target_width=100.,measurements=[dict(id='a',text='first',width=100.),dict(id='b',text='second',width=width)])

    def test_stable_zero_tie_and_confidence_selection(self):
        self.assertEqual(replay(self.loop,self.case(),self.native(),{},0.)[0]['candidate'],'first')
        prior=replay(self.loop,self.case(),self.native(),dict(a=0.,b=1.),10.)
        self.assertEqual(prior[0]['candidate'],'second');self.assertEqual(prior[0]['confidence'],10.)

    def test_native_gate_applies_to_selected_raw_error(self):
        # A large prior can select a worse geometric fit and cause rejection,
        # even though another candidate has zero width error.
        self.assertEqual(replay(self.loop,self.case(),self.native(200.),dict(a=0.,b=1.),1000.),[])
        edge=dict(target_width=70.,measurements=[dict(id='a',text='first',width=100.)])
        self.assertEqual(replay(self.loop,self.case(),edge,{},0.),[])


if __name__=='__main__':unittest.main()
