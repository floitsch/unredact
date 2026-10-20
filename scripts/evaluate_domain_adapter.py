# Copyright (C) 2026 Toit contributors.
"""Separate adapted proposal and ranking effects on the fixed eight-case cohort."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from evaluate_corpus_model_union import load,write,digest
from evaluate_span_ranking import rank
from span_context_provider import prepare,score_items,verify_model


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--adapter',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--model-dir',type=Path,default=Path('runs/span-model-001/t5-base'))
    a=p.parse_args()
    a.output.mkdir(parents=True,exist_ok=False)
    source=Path('runs/combined-ranking-holdout-001/context-evaluation')
    union=Path('runs/corpus-model-union-001')
    corpus=source/'open-candidates.json'
    original_candidates=load(corpus)['candidates']
    suite=load(source.parent/'suite.json')
    binary=Path('target/release/unredact').resolve()
    model_python=Path('runs/neural-venv/bin/python').absolute()
    replay_python=Path('runs/baseline-venv/bin/python').absolute()
    replay=Path('scripts/evaluate_bounded_controls.py').resolve()
    proposal=Path('scripts/span_proposal_provider.py').resolve()
    artifacts=[Path(__file__),Path('scripts/span_adapter.py'),Path('scripts/span_context_provider.py'),
        Path('scripts/evaluate_corpus_model_union.py'),proposal,replay,binary,corpus,source.parent/'suite.json',
        a.adapter/'manifest.json',a.adapter/'adapter.safetensors',a.adapter.parent/'dataset.json',
        a.adapter.parent/'experiment-plan.json',*Path('src').glob('*.rs')]
    for i in range(8):
        artifacts += [source/f'case-{i:03}/open-weight-0.json',union/f'case-{i:03}/combined-scores.json',
            source.parent/'comparison'/f'case-{i:03}-repeat-00-tool-00/work/project/project.json']
    frozen={str(path):digest(path) for path in artifacts}
    pin=verify_model(a.model_dir)
    write(a.output/'frozen.json',dict(files=frozen,base_model=pin,weights=[0,1],proposal_beams=32,
        scope='All eight reused locations; train/validation exclude all original test pages. Fixed checkpoint selected by validation token NLL only. Development evaluation, not a new holdout.'))
    os.environ['HF_HUB_OFFLINE']='1'
    os.environ['TRANSFORMERS_OFFLINE']='1'
    os.environ['TOKENIZERS_PARALLELISM']='false'
    import torch
    from transformers import T5TokenizerFast,T5ForConditionalGeneration
    from span_adapter import load as attach
    torch.set_num_threads(4)
    tokenizer=T5TokenizerFast.from_pretrained(a.model_dir,local_files_only=True)
    base_model=T5ForConditionalGeneration.from_pretrained(a.model_dir,local_files_only=True,use_safetensors=True).eval()
    adapted_model=T5ForConditionalGeneration.from_pretrained(a.model_dir,local_files_only=True,use_safetensors=True).eval()
    adapter_pin=attach(adapted_model,a.adapter,pin)
    rows=[]

    def run(command,stderr):
        result=subprocess.run(list(map(str,command)),capture_output=True,text=True,timeout=330)
        stderr.write_text(result.stderr)
        if result.returncode:raise RuntimeError(result.stderr)

    def score(request,model):
        scores=[]
        for start in range(0,len(request['items']),2048):
            prepared,contexts=prepare({**request,'items':request['items'][start:start+2048]},tokenizer,64)
            score_items(prepared,contexts,tokenizer,model,torch,16)
            scores.extend(prepared)
        return scores

    for i,case in enumerate(suite['cases']):
        directory=a.output/f'case-{i:03}';directory.mkdir()
        project=source.parent/'comparison'/f'case-{i:03}-repeat-00-tool-00/work/project'
        prior=load(source/f'case-{i:03}/open-weight-0.json')
        expansion_path=directory/'expansion.json'
        run([binary,'expand','--project',project,'--group','comparison','--candidates',corpus,
            '--paragraph-context','--max-proposals',32,'--provider',model_python,
            '--provider-arg',proposal,'--provider-arg=--model-dir','--provider-arg',a.model_dir,
            '--provider-arg=--adapter','--provider-arg',a.adapter,
            '--output',expansion_path],directory/'expansion.stderr')
        expanded=load(expansion_path)
        assert expanded['initial']==prior['physical'] and expanded['contexts']==prior['contexts']
        assert expanded['candidates'][:len(original_candidates)]==original_candidates
        assert expanded['provider']['response']['provenance']['domain_adapter']==adapter_pin
        assert set(expanded['provider']['request'])=={'schema_version','task','contexts','limits','surface_kind'}
        physical={c['id']:c for c in expanded['expanded']['candidates']}
        items=[dict(id=c['id'],target=o['target'],text=o['surface']) for c in physical.values() if c['compatible'] for o in c['observations']]
        request=dict(schema_version=1,task='score_context_candidates',contexts=expanded['contexts'],items=items)
        cached=load(union/f'case-{i:03}/combined-scores.json')
        assert cached['model']==pin and cached['request']['contexts']==request['contexts']
        texts={(x['id'],x['target']):x['text'] for x in cached['request']['items']}
        old_scores={(s['id'],s['target']):s for s in cached['scores']}
        base_scores=[];fresh=[]
        for item in items:
            key=item['id'],item['target']
            if key in old_scores:
                assert texts[key]==item['text'];base_scores.append(old_scores[key])
            else:fresh.append(item)
        base_scores += score({**request,'items':fresh},base_model)
        adapted_scores=score(request,adapted_model)
        for name,scores in [('base',base_scores),('adapted',adapted_scores)]:
            write(directory/f'{name}-scores.json',dict(request=request,scores=scores,model=pin,
                domain_adapter=adapter_pin if name=='adapted' else None))
        # Exercise the real adapter-aware scorer executable as well as the
        # in-process evaluation path. Compare three measured likelihoods.
        if i==0:
            smoke={**request,'items':items[:3]}
            smoke_request=directory/'scorer-smoke-request.json';write(smoke_request,smoke)
            command=[str(model_python),'scripts/span_context_provider.py','--model-dir',str(a.model_dir),'--adapter',str(a.adapter)]
            result=subprocess.run(command,input=json.dumps(smoke),capture_output=True,text=True,timeout=330)
            (directory/'scorer-smoke.stderr').write_text(result.stderr)
            assert result.returncode==0,result.stderr
            reply=json.loads(result.stdout);write(directory/'scorer-smoke-response.json',reply)
            assert reply['domain_adapter']==adapter_pin
            expected={(s['id'],s['target']):s for s in adapted_scores}
            for item in reply['scores']:
                other=expected[item['id'],item['target']]
                assert item['token_count']==other['token_count']
                assert abs(item['token_log_score_sum']-other['token_log_score_sum'])<1e-4
        results={}
        for track,vocabulary,cache,expected_physics in [
            ('adapted_proposals_base_rank',expansion_path,directory/'base-scores.json',expanded['expanded']),
            ('adapted_proposals_adapted_rank',expansion_path,directory/'adapted-scores.json',expanded['expanded']),
            ('corpus_adapted_rank',corpus,directory/'adapted-scores.json',prior['physical'])]:
            reports={}
            for weight in [0,1]:
                path=directory/f'{track}-weight-{weight}.json'
                run([binary,'context-rank','--project',project,'--group','comparison','--candidates',vocabulary,
                    '--paragraph-context','--sequence-score','--physical-weight',weight,
                    '--provider',replay_python,'--provider-arg',replay,'--provider-arg=--replay',
                    '--provider-arg',cache.resolve(),'--output',path],directory/f'{track}-weight-{weight}.stderr')
                reports[weight]=load(path)
                assert reports[weight]['physical']==expected_physics and reports[weight]['contexts']==prior['contexts']
            truth=next(c['surfaces'][0]['text'] for c in case['input']['candidates'] if c['id']==case['truth_id'])
            candidates=original_candidates if track=='corpus_adapted_rank' else expanded['candidates']
            ids={c['id'] for c in candidates if c['surfaces'][0]['text']==truth}
            allowed={c['id'] for c in expected_physics['candidates'] if c['compatible']}
            results[track]=dict(truth_available=bool(ids),truth_compatible=bool(ids&allowed),
                language_rank=rank(reports[0]['ranked_groups'],ids),combined_rank=rank(reports[1]['ranked_groups'],ids))
        row=dict(case=case['id'],proposals=len(expanded['proposals']),added=len(expanded['added_candidate_ids']),
            compatible_added=len(expanded['compatible_added_candidate_ids']),truth_generated=any(p['text']==truth for p in expanded['proposals']),results=results)
        write(directory/'evaluation.json',row);rows.append(row);print(json.dumps(row),flush=True)
    assert all(digest(path)==sha for path,sha in frozen.items())
    assert verify_model(a.model_dir)==pin
    totals={}
    for track in rows[0]['results']:
        values=[r['results'][track] for r in rows]
        totals[track]=dict(cases=len(values),truth_available=sum(v['truth_available'] for v in values),
            truth_compatible=sum(v['truth_compatible'] for v in values),
            language_first=sum(v['language_rank']==[1,1] for v in values),combined_first=sum(v['combined_rank']==[1,1] for v in values))
    write(a.output/'summary.json',dict(totals=totals,rows=rows,all_frozen_hashes_verified=True,adapter=adapter_pin,
        interpretation='Development follow-up, all eight previously scored locations retained. Domain adapter selected on separate visible pages by token NLL only. Fixed beams and weights. No upstream-AI or independent-document superiority claim.'))
    print(json.dumps(totals),flush=True)


if __name__=='__main__':main()
