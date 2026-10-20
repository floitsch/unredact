# Copyright (C) 2026 Toit contributors.
"""Apply identical T5 scores from both contexts to saved joint geometric sets."""
import argparse
import hashlib
import json
import os
from pathlib import Path
from evaluate_common_language import truth_rank
from span_context_provider import prepare, score_items, verify_model


def load(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value,indent=2)+'\n')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--local',type=Path,required=True)
    p.add_argument('--external',type=Path,required=True)
    p.add_argument('--suite',type=Path,required=True)
    p.add_argument('--model-dir',type=Path,default=Path('runs/span-model-001/t5-base'))
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    a.output.mkdir(parents=True,exist_ok=False)
    suite=load(a.suite)
    cases={c['id']:c for c in suite['cases'] if c['id'].startswith('joint-present-')}
    local={}
    for file in a.local.glob('*/execution.json'):
        row=load(file)
        if row['case'] in cases and row['adapter']=='width-fringe-paragraph-sequence-t5':
            assert row['case'] not in local
            local[row['case']]=(row,file.parent)
    assert set(local)==set(cases) and len(cases)==6
    external=load(a.external/'comparison.json')
    artifacts=[Path(__file__),Path(__file__).with_name('evaluate_common_language.py'),
        Path(__file__).with_name('span_context_provider.py'),a.suite,a.external/'comparison.json']
    for _,directory in local.values():
        artifacts += [directory/'execution.json',directory/'request.json',directory/'work/context-result.json']
    frozen={str(path):digest(path) for path in artifacts}
    model_pin=verify_model(a.model_dir)
    write(a.output/'frozen.json',dict(files=frozen,model=model_pin,
        scope='Six reused joint cases. Every candidate gets the same two-context T5 mean score for all tools. Supplied links and masks; no upstream AI or new holdout claim. Unsupported external cases retained.'))
    os.environ['HF_HUB_OFFLINE']='1'
    os.environ['TRANSFORMERS_OFFLINE']='1'
    os.environ['TOKENIZERS_PARALLELISM']='false'
    import torch
    from transformers import T5TokenizerFast,T5ForConditionalGeneration
    torch.set_num_threads(4)
    tokenizer=T5TokenizerFast.from_pretrained(a.model_dir,local_files_only=True)
    model=T5ForConditionalGeneration.from_pretrained(a.model_dir,local_files_only=True,use_safetensors=True).eval()
    results=[]
    for identity in sorted(cases):
        execution,directory=local[identity]
        assert not execution['error'] and execution['result']['status']=='ok'
        native=load(directory/'work/context-result.json')
        data=load(directory/'request.json')['input']
        candidates=data['candidates']
        assert len(native['contexts'])==2 and native['physical']['independent_observations']==2
        assert all(len(c['surfaces'])==1 for c in candidates)
        items=[dict(id=c['id'],target=target,text=c['surfaces'][0]['text'])
               for target in native['contexts'] for c in candidates]
        request=dict(schema_version=1,task='score_context_candidates',contexts=native['contexts'],items=items)
        prepared,used=prepare(request,tokenizer,64)
        stats=score_items(prepared,used,tokenizer,model,torch,16)
        by_id={c['id']:[] for c in candidates}
        for item in prepared:
            by_id[item['id']].append(item['token_log_score_sum'])
        assert all(len(v)==2 for v in by_id.values())
        scores={key:sum(values)/2 for key,values in by_id.items()}
        allowed={c['id'] for c in native['physical']['candidates'] if c['compatible']}
        assert {r['id'] for r in native['ranked']}==allowed
        assert all(abs(r['mean_sequence_log_score']-scores[r['id']])<1e-4 for r in native['ranked'])
        sets={'language_only':set(scores),'bounded_local_geometry':allowed}
        unsupported={}
        entries=[r for r in external['executions'] if r['case']==identity]
        assert len(entries)==2 and len({r['adapter'] for r in entries})==2
        for row in entries:
            assert row['input_sha256']==execution['input_sha256']
            assert not row['error']
            if row['result']['status']!='ok':
                unsupported[row['adapter']]=dict(status=row['result']['status'],reason=row['result'].get('reason'))
                continue
            assert row['result']['compatible'] is not None
            sets[row['adapter']]=set(row['result']['compatible'])
            assert sets[row['adapter']].issubset(scores)
        # Truth enters only after scoring and geometric filters are fixed.
        truth=cases[identity]['truth_id']
        graded={name:dict(status='ok',truth_rank=truth_rank({k:scores[k] for k in ids},truth),
                         compatible=len(ids),truth_compatible=truth in ids) for name,ids in sets.items()}
        graded.update(unsupported)
        assert graded['bounded_local_geometry']['truth_rank']==execution['metrics']['truth_rank']
        row=dict(case=identity,results=graded,input_sha256=execution['input_sha256'])
        write(a.output/f'{identity}.json',dict(**row,request=request,scores=prepared,contexts_used=used,forward_states=stats))
        results.append(row)
        print(json.dumps(row),flush=True)
    assert all(digest(Path(path))==sha for path,sha in frozen.items())
    assert verify_model(a.model_dir)==model_pin
    totals={}
    for row in results:
        for name,result in row['results'].items():
            t=totals.setdefault(name,dict(cases=0,successful=0,unique_first=0,truth_retained=0))
            t['cases']+=1
            t['successful']+=result['status']=='ok'
            t['unique_first']+=result.get('truth_rank')==[1,1]
            t['truth_retained']+=result.get('truth_compatible',False)
    write(a.output/'summary.json',dict(totals=totals,cases=results,all_frozen_hashes_verified=True))


if __name__=='__main__':
    main()
