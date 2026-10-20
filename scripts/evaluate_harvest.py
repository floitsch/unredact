# Copyright (C) 2026 Toit contributors.
"""Harvest visible corpus text, then grade independent masked locations.

Labels enter only after vocabulary construction and ranking. These reused
controlled masks test retrieval from visible repetitions, not lexical holdouts.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from evaluate_span_ranking import rank


def load(path):
    return json.loads(Path(path).read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value,indent=2)+'\n')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--binary',type=Path,default=Path('target/release/unredact'))
    p.add_argument('--python',type=Path,default=Path('runs/neural-venv/bin/python'))
    p.add_argument('--model-dir',type=Path,default=Path('runs/span-model-001/t5-base'))
    p.add_argument('--max-candidates',type=int,default=10_000)
    a=p.parse_args()
    manifest=load(a.manifest)
    a.output.mkdir(parents=True,exist_ok=False)
    scripts=['src/harvest.rs','src/main.rs','src/lib.rs','src/contextual.rs','src/context_batches.rs',
             'src/calibration.rs','src/solver.rs','scripts/evaluate_harvest.py',
             'scripts/evaluate_span_ranking.py','scripts/span_context_provider.py']
    frozen={str(p):digest(p) for p in [a.manifest,a.binary,*map(Path,scripts)]}
    for corpus in manifest['corpora'].values():
        for directory in corpus:
            path=Path(directory)/'project.json'
            frozen[str(path)]=digest(path)
    write(a.output/'frozen.json',dict(files=frozen,design=manifest['design']))
    corpora={}
    for name,directories in manifest['corpora'].items():
        path=a.output/f'{name}-candidates.json'
        cmd=[str(a.binary.resolve()),'harvest','--max-candidates',str(a.max_candidates),'--output',str(path)]
        for directory in directories:
            cmd+=['--project',directory]
        proc=subprocess.run(cmd,capture_output=True,text=True,timeout=300)
        (a.output/f'{name}-harvest.stderr').write_text(proc.stderr)
        if proc.returncode:
            raise RuntimeError(proc.stderr)
        report=load(path)
        corpora[name]=dict(path=str(path),sha256=digest(path),counts=report['counts'],
                          config=report['config'],candidate_count=len(report['candidates']))
        print(name,report['counts'],flush=True)
    write(a.output/'corpora.json',corpora)
    # Every corpus list is frozen before looking at any label to grade it.
    rows=[]
    for index,case in enumerate(manifest['cases']):
        directory=a.output/f'case-{index:03}'
        directory.mkdir()
        vocabulary=corpora[case['corpus']]['path']
        command=[str(a.binary.resolve()),'context-rank','--project',case['project'],
            '--group','comparison','--candidates',vocabulary,'--paragraph-context','--sequence-score',
            '--provider',str(a.python),'--provider-arg',str(Path(__file__).with_name('span_context_provider.py').resolve()),
            '--provider-arg=--model-dir','--provider-arg',str(a.model_dir),'--output',str(directory/'ranking.json')]
        proc=subprocess.run(command,capture_output=True,text=True,timeout=330)
        (directory/'stderr.txt').write_text(proc.stderr)
        row=dict(case=case['id'],cohort=case['cohort'],corpus=case['corpus'],command=command,
                 status='error' if proc.returncode else 'ok')
        candidates=load(vocabulary)['candidates']
        truths={c['id'] for c in candidates if any(s['text'] in case['truth_texts'] for s in c['surfaces'])}
        row.update(truth_available=bool(truths),truth_ids=sorted(truths),vocabulary_size=len(candidates))
        if not proc.returncode:
            report=load(directory/'ranking.json')
            allowed={c['id'] for c in report['physical']['candidates'] if c['compatible']}
            row.update(truth_compatible=bool(truths & allowed),truth_rank=rank(report['ranked_groups'],truths),
                       compatible_candidates=len(allowed))
        rows.append(row)
        write(directory/'evaluation.json',row)
        print(json.dumps({k:v for k,v in row.items() if k!='command'}),flush=True)
    assert all(digest(path)==sha for path,sha in frozen.items())
    assert all(digest(c['path'])==c['sha256'] for c in corpora.values())
    totals={}
    for row in rows:
        t=totals.setdefault(row['cohort'],dict(cases=0,successful=0,truth_available=0,truth_compatible=0,unique_first=0))
        t['cases']+=1;t['successful']+=row['status']=='ok'
        for key in ['truth_available','truth_compatible']:
            t[key]+=row.get(key,False)
        t['unique_first']+=row.get('truth_rank')==[1,1]
    write(a.output/'summary.json',dict(totals=totals,cases=rows,corpora=corpora,all_frozen_hashes_verified=True,
        interpretation='Visible-corpus retrieval on reused masked locations. Correct text is not supplied to harvesting or ranking. Unmasked repetitions are allowed and recorded. Corpus candidate counts differ from model proposal budgets; no equal-budget or unseen-document claim.'))


if __name__=='__main__':
    main()
