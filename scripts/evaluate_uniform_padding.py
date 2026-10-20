# Copyright (C) 2026 Toit contributors.
"""Paired, frozen development ablation of an explicit uniform padding prior.

Uses cached OCR/profiles from completed broad comparisons. It does not use hidden
padding values, choose cases, tune prior parameters, or read test-case outcomes.
"""
import argparse
import copy
import hashlib
import json
import re
from pathlib import Path
import subprocess


def load(p):return json.loads(Path(p).read_text())
def digest(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,v):Path(p).write_text(json.dumps(v,indent=2)+'\n')


def assess(report,truth):
    found=next((c for c in report['candidates'] if c['id']==truth),None)
    return dict(unique_correct_first=bool(found and found['evidence_rank_min']==found['evidence_rank_max']==1),
                truth_best_group=bool(found and found['evidence_rank_min']==1),truth_compatible=bool(found and found['compatible']),
                truth_rank=[found['evidence_rank_min'],found['evidence_rank_max']] if found else None,
                compatible_count=report['compatible_candidates'],status=report['status'])


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--plan',type=Path,required=True);p.add_argument('--binary',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan);base=Path(plan['baseline']);out=a.plan.parent/'evaluation'
    assert digest(a.binary)==load(a.plan.parent/'binary.json')['sha256']
    assert all(digest(p)==h for p,h in load(a.plan.parent/'source-hashes.json').items())
    # Gating occurs before any case execution or output directory creation.
    reports={v:load(base/v/'comparison.json') for v in plan['availability']}
    baseline_freeze=load(base/'freeze.json')
    assert all(digest(p)==s for p,s in baseline_freeze.items()),'Baseline files changed'
    assert all(not p.get('error') for r in reports.values() for p in r['provenance'])
    old_manifest=load(base/'adapters.json');local=old_manifest['adapters'][0]['id']
    original=Path('runs/multidoc-benchmark-001');natural=Path('runs/multidoc-natural-001')
    labels=load(original/'controller/labels.json')['labels'];metadata={c['id']:r for r in labels for c in r['cases']}
    suites={}
    files={str(a.plan.resolve()):digest(a.plan),str(a.binary.resolve()):digest(a.binary),str(Path(__file__).resolve()):digest(__file__)}
    for v in plan['availability']:
        for bank,root in [('original_dictionary',original),('natural_visible_development',natural)]:
            path=root/f'development-opaque-{v}.json';suite=load(path);suites[bank,v]=suite
            assert len(suite['cases'])==plan['source_cases']==96
            files[str(path.resolve())]=digest(path)
            if bank=='original_dictionary':assert reports[v]['suite_sha256']==digest(path)
        entries=reports[v]['executions']
        assert len(entries)==96*3 and len({(e['adapter'],e['case'],e['repetition']) for e in entries})==len(entries)
        assert {e['case'] for e in entries}=={c['id'] for c in suites['original_dictionary',v]['cases']}
        for index,case in enumerate(suites['original_dictionary',v]['cases']):
            assert metadata[case['id']]['split']=='development'
            reference=next(e for e in entries if e['adapter']==local and e['case']==case['id'])
            if reference['error'] or reference['result']['status']!='ok':continue
            work=base/v/f'case-{index:03}-repeat-00-tool-00/work'
            for path in [work/'project/project.json',work/'candidates.json',work/'native-result.json']:
                files[str(path.resolve())]=digest(path)
    out.mkdir(parents=True,exist_ok=False)
    write(out/'freeze.json',dict(files=files,baseline_files=baseline_freeze))
    rows=[]
    for availability in plan['availability']:
        for index,case in enumerate(suites['original_dictionary',availability]['cases']):
            directory=out/f'{availability}-{index:03}';directory.mkdir()
            label=metadata[case['id']]
            row=dict(case=case['id'],document=label['document'],source_group=label['source_group'],kind=label['kind'],availability=availability,tracks={})
            reference=next(e for e in reports[availability]['executions'] if e['adapter']==local and e['case']==case['id'])
            if reference['error'] or reference['result']['status']!='ok':
                row['source_error']=reference['error'] or reference['result'].get('reason');rows.append(row);write(directory/'result.json',row);continue
            work=base/availability/f'case-{index:03}-repeat-00-tool-00/work';source=load(work/'project/project.json')
            raw_source=(work/'project/project.json').read_text()
            assert all(t['evidence']['kind']=='opaque' for t in source['targets'])
            for bank in plan['vocabularies']:
                matching=suites[bank,availability]['cases'][index]
                assert matching['id']==case['id'] and matching['truth_id']==case['truth_id']
                assert {k:v for k,v in matching['input'].items() if k!='candidates'}=={k:v for k,v in case['input'].items() if k!='candidates'}
                candidates=directory/f'{bank}-candidates.json';write(candidates,dict(candidates=matching['input']['candidates']))
                outputs={};metrics={}
                for mode in plan['modes']:
                    data=copy.deepcopy(source);project=directory/f'{bank}-{mode}';project.mkdir()
                    if mode=='uniform_padding_prior':
                        for target in data['targets']:target['evidence']['kind']='opaque_uniform'
                    # Preserve original decimal strings: JSON float round-trips
                    # must not become a confound in a scoring-only ablation.
                    raw=raw_source
                    if mode=='uniform_padding_prior':
                        raw=re.sub(r'("kind"\s*:\s*)"opaque"',r'\1"opaque_uniform"',raw)
                    assert json.loads(raw)==data
                    (project/'project.json').write_text(raw)
                    for page in source['pages']:
                        image=Path(page['image'])
                        if not image.is_absolute():
                            asset=image.parts[0]
                            if not (project/asset).exists():
                                original_asset=(work/'project'/asset).resolve()
                                (project/asset).symlink_to(original_asset,target_is_directory=original_asset.is_dir())
                    path=project/'physical.json'
                    result=subprocess.run([str(a.binary.resolve()),'solve','--project',str(project),'--group','comparison',
                        '--candidates',str(candidates),'--limit',str(len(matching['input']['candidates'])),'--output',str(path)],capture_output=True,text=True,timeout=120)
                    (project/'stderr.txt').write_text(result.stderr)
                    if result.returncode:
                        metrics[mode]=dict(execution_status='error',error=result.stderr);continue
                    report=load(path);outputs[mode]=report
                    if bank=='original_dictionary' and mode=='existing_profile_cost':
                        assert report==load(work/'native-result.json'),'Default scoring no longer reproduces the frozen baseline'
                    metrics[mode]=dict(execution_status='ok',**assess(report,case['truth_id']))
                if len(outputs)==2:
                    before={c['id']:c for c in outputs['existing_profile_cost']['candidates']}
                    after={c['id']:c for c in outputs['uniform_padding_prior']['candidates']}
                    assert before.keys()==after.keys()
                    metrics['paired_diagnostics']=dict(changed_fit_candidates=sum(before[i]['observations'][0]['fit_index']!=after[i]['observations'][0]['fit_index'] for i in before),
                        changed_compatibility_candidates=sum(before[i]['compatible']!=after[i]['compatible'] for i in before),
                        candidates=len(before))
                row['tracks'][bank]=metrics
            rows.append(row);write(directory/'result.json',row)
            print(availability,index+1,'of96 completed',flush=True)
    assert all(digest(p)==s for p,s in files.items())
    assert all(digest(p)==s for p,s in baseline_freeze.items())
    write(out/'summary.json',dict(rows=rows,all_frozen_files_verified=True,
        interpretation='All96 development spans and matched absent controls. Original dictionary and natural distractor banks scored separately; no cross-bank baseline comparison. Default-cost replay must exactly reproduce original native reports. This tests an explicit uniform prior using cached OCR/profiles, not a fresh end-to-end independent test. No hidden padding or test outcome used.'))


if __name__=='__main__':main()
