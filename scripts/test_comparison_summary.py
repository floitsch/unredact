# Copyright (C) 2026 Toit contributors.
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT=Path(__file__).with_name('summarize_comparisons.py')


def report(adapter='a'):
    return dict(suite_sha256='suite',summary={adapter:dict(mean_elapsed_seconds=1.,
        unique_correct_first=1,truth_present_successes=1,truth_absent_successes=0,
        truth_compatible=1,compatible_reported=1,no_compatible_when_truth_absent=0,failed=0)},
        executions=[dict(adapter=adapter,case='case',repetition=0,input_sha256='input',
                         result={'status':'ok'},metrics={'correct':True},error=None)])


class SummaryTests(unittest.TestCase):
    def run_reports(self, reports):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);args=[sys.executable,str(SCRIPT)]
            for i,r in enumerate(reports):
                path=root/f'{i}.json';path.write_text(json.dumps(r));args+=['--comparison',str(path)]
            output=root/'summary.json';args+=['--output',str(output)]
            run=subprocess.run(args,capture_output=True,text=True)
            return run,json.loads(output.read_text()) if output.exists() else None

    def test_repeated_results_are_checked_not_counted_as_new_trials(self):
        a=report();b=copy.deepcopy(a);b['summary']['a']['mean_elapsed_seconds']=2.
        run,out=self.run_reports([a,b,report('b')])
        self.assertEqual(run.returncode,0,run.stderr)
        self.assertEqual(len(out['identical_repeated_results']),1)
        self.assertEqual(out['configurations']['a']['summary']['truth_present_successes'],1)
        self.assertEqual(len(out['configurations']['a']['runs']),2)

    def test_incompatible_evidence_is_rejected(self):
        variants=[]
        a=report('b');a['suite_sha256']='different';variants.append(a)
        a=report('b');a['executions'][0]['input_sha256']='different';variants.append(a)
        a=report('b');a['executions'][0]['case']='different';variants.append(a)
        a=report();a['executions'][0]['result']={'status':'error'};variants.append(a)
        for other in variants:
            with self.subTest(other=other):
                run,out=self.run_reports([report(),other]);self.assertNotEqual(run.returncode,0);self.assertIsNone(out)


if __name__=='__main__':unittest.main()
