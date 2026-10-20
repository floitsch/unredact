# Copyright (C) 2026 Toit contributors.
import unittest

from prepare_natural_candidates import draw_vocabulary,visible_phrases


class NaturalCandidateTests(unittest.TestCase):
    def test_mask_and_bad_token_are_phrase_barriers(self):
        data=dict(text=['left','secret','right','next','bad','tail'],left=[0,25,60,100,135,170],top=[10]*6,
                  width=[20,30,30,30,30,30],height=[20]*6,conf=[95,99,95,95,20,95],
                  block_num=[1]*6,par_num=[1]*6,line_num=[1]*6)
        actual={x['text'] for x in visible_phrases(data,[(24,8,56,32)])}
        self.assertEqual(actual,{'left','right','next','tail','right next'})
        # OCR skipped the masked word entirely: the geometric mask still bars a phrase.
        skipped={k:[v[i] for i in [0,2]] for k,v in data.items()}
        self.assertNotIn('left right',{x['text'] for x in visible_phrases(skipped,[(24,8,56,32)])})

    def test_closed_injection_does_not_change_class_balance_or_open_draw(self):
        pools={1:{'alpha','beta','gamma','delta'},2:{'one alpha','one beta','one gamma','one delta'}}
        open_a,closed_a=draw_vocabulary(pools,'missing',4,18)
        open_b,closed_b=draw_vocabulary(pools,'absent phrase',4,18)
        self.assertEqual(open_a,open_b)
        self.assertIn('missing',closed_a);self.assertIn('absent phrase',closed_b)
        for result in (closed_a,closed_b):
            self.assertEqual(len(result),len(set(result)))
            self.assertEqual([sum(len(t.split())==n for t in result) for n in (1,2)],[2,2])
        with self.assertRaises(ValueError):draw_vocabulary(pools,'missing',12,18)


if __name__=='__main__':unittest.main()
