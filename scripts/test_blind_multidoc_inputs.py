# Copyright (C) 2026 Toit contributors.
import copy
import unittest

from blind_multidoc_inputs import replace_absent


class BlindingTests(unittest.TestCase):
    def test_absent_replacement_preserves_class_balance_and_does_not_insert_truth(self):
        candidates=[dict(id='a',canonical_name='alpha'),dict(id='b',canonical_name='two words'),dict(id='c',canonical_name='other phrase')]
        original=copy.deepcopy(candidates)
        pool=['truth','alpha','beta','gamma','two words','other phrase','new phrase']
        result,replacement=replace_absent(candidates,'truth',pool,'span')
        self.assertEqual(candidates,original)
        self.assertEqual([sum(len(c['canonical_name'].split())==n for c in result) for n in (1,2)],[2,2])
        self.assertNotIn('truth',[c['canonical_name'] for c in result])
        self.assertEqual((result,replacement),replace_absent(candidates,'truth',pool[::-1],'span'))
        self.assertIn(replacement['canonical_name'],['beta','gamma'])
        with self.assertRaises(ValueError):replace_absent(candidates,'truth',['truth','alpha'],'span')


if __name__=='__main__':unittest.main()
