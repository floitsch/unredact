# Copyright (C) 2026 Toit contributors.
"""Evaluate visible-corpus candidates plus fixed-budget T5 proposals.

No grading label enters a proposal or ranking request. Existing inference is
reused only after context, candidate surface, model and physical checks. New
compatible proposals are scored by the same pinned model. Both ordering modes
execute through Rust using a strict cached-score provider.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from evaluate_span_ranking import rank
from span_context_provider import prepare, score_items, verify_model


def load(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--model-dir', type=Path, default=Path('runs/span-model-001/t5-base'))
    args = parser.parse_args()
    plan = load(args.plan)
    out = args.plan.parent
    source = Path(plan['source'])
    corpus_path = Path(plan['corpus'])
    assert digest(corpus_path) == plan['corpus_sha256']
    base_candidates = load(corpus_path)['candidates']
    suite = load(source.parent/'suite.json')
    assert len(suite['cases']) == plan['cases'] == 8
    binary = Path('target/release/unredact').resolve()
    model_python = Path('runs/neural-venv/bin/python').absolute()
    replay_python = Path('runs/baseline-venv/bin/python').absolute()
    proposal_provider = Path('scripts/span_proposal_provider.py').resolve()
    replay = Path('scripts/evaluate_bounded_controls.py').resolve()
    paths = [args.plan,Path(__file__),binary,corpus_path,proposal_provider,replay,
             Path('scripts/span_context_provider.py'),source.parent/'suite.json',source/'summary.json',
             *Path('src').glob('*.rs')]
    for index in range(8):
        paths += [source/f'case-{index:03}'/name for name in ['open-scores.json','open-weight-0.json','open-weight-1.json']]
        paths += [source.parent/'comparison'/f'case-{index:03}-repeat-00-tool-00/work/project/project.json']
    frozen = {str(p):digest(p) for p in paths}
    pin = verify_model(args.model_dir)
    write(out/'frozen.json',dict(files=frozen,model=pin,scope=plan['scope']))
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    os.environ['TOKENIZERS_PARALLELISM'] = 'false'
    import torch
    from transformers import T5TokenizerFast, T5ForConditionalGeneration
    torch.set_num_threads(4)
    tokenizer = T5TokenizerFast.from_pretrained(args.model_dir,local_files_only=True)
    model = T5ForConditionalGeneration.from_pretrained(args.model_dir,local_files_only=True,use_safetensors=True).eval()

    def run(command, stderr):
        result = subprocess.run(list(map(str,command)),capture_output=True,text=True,timeout=330)
        stderr.write_text(result.stderr)
        if result.returncode:
            raise RuntimeError(result.stderr)

    rows = []
    for index, case in enumerate(suite['cases']):
        directory = out/f'case-{index:03}'
        directory.mkdir(exist_ok=False)
        project = source.parent/'comparison'/f'case-{index:03}-repeat-00-tool-00/work/project'
        original = load(source/f'case-{index:03}/open-weight-0.json')
        old_cache = load(source/f'case-{index:03}/open-scores.json')
        assert old_cache['model'] == pin
        assert original['contexts'] == old_cache['request']['contexts']
        expansion_path = directory/'expansion.json'
        command = [binary,'expand','--project',project,'--group','comparison','--candidates',corpus_path,
            '--paragraph-context','--max-proposals',plan['max_proposals'],'--provider',model_python,
            '--provider-arg',proposal_provider,'--provider-arg=--model-dir','--provider-arg',args.model_dir,
            '--provider-arg=--beams','--provider-arg',plan['beams'],
            '--provider-arg=--tokens','--provider-arg',plan['max_new_tokens'],
            '--provider-arg=--context-tokens','--provider-arg',plan['context_tokens_per_side'],
            '--output',expansion_path]
        run(command,directory/'expansion.stderr')
        expanded = load(expansion_path)
        assert expanded['initial'] == original['physical']
        assert expanded['contexts'] == original['contexts']
        assert expanded['candidates'][:len(base_candidates)] == base_candidates
        assert set(expanded['provider']['request']) == {'schema_version','task','contexts','limits','surface_kind'}
        assert expanded['provider']['response']['provenance']['model'] == pin
        old_physical = {c['id']:c for c in original['physical']['candidates']}
        new_physical = {c['id']:c for c in expanded['expanded']['candidates']}
        for identity, prior in old_physical.items():
            for key in prior:
                if key not in ['evidence_rank_min','evidence_rank_max']:
                    assert new_physical[identity][key] == prior[key], (identity,key)
        cached_text = {(i['id'],i['target']):i['text'] for i in old_cache['request']['items']}
        cached_scores = {(s['id'],s['target']):s for s in old_cache['scores']}
        assert cached_text.keys() == cached_scores.keys()
        items = [dict(id=c['id'],target=o['target'],text=o['surface'])
                 for c in expanded['expanded']['candidates'] if c['compatible'] for o in c['observations']]
        scores, fresh = [], []
        for item in items:
            key = item['id'],item['target']
            if key in cached_scores:
                assert cached_text[key] == item['text']
                scores.append(cached_scores[key])
            else:
                assert item['id'] in expanded['compatible_added_candidate_ids']
                fresh.append(item)
        diagnostics = []
        for start in range(0,len(fresh),2048):
            request = dict(schema_version=1,task='score_context_candidates',contexts=expanded['contexts'],items=fresh[start:start+2048])
            prepared, used = prepare(request,tokenizer,plan['context_tokens_per_side'])
            stats = score_items(prepared,used,tokenizer,model,torch,16)
            scores.extend(prepared)
            diagnostics.append(dict(contexts_used=used,forward_states=stats))
        cache = directory/'combined-scores.json'
        write(cache,dict(request=dict(schema_version=1,task='score_context_candidates',contexts=expanded['contexts'],items=items),
            scores=scores,model=pin,diagnostics=diagnostics,reused_scores=len(items)-len(fresh),new_scores=len(fresh),
            original_cache=str(source/f'case-{index:03}/open-scores.json'),original_cache_sha256=digest(source/f'case-{index:03}/open-scores.json')))
        rankings = {}
        for weight in plan['ranking_weights']:
            path = directory/f'weight-{weight}.json'
            run([binary,'context-rank','--project',project,'--group','comparison','--candidates',expansion_path,
                '--paragraph-context','--sequence-score','--physical-weight',weight,
                '--provider',replay_python,'--provider-arg',replay,'--provider-arg=--replay',
                '--provider-arg',cache.resolve(),'--output',path],directory/f'weight-{weight}.stderr')
            result = load(path)
            assert result['physical'] == expanded['expanded'] and result['contexts'] == expanded['contexts']
            old_ranking = load(source/f'case-{index:03}/open-weight-{weight}.json')
            old_entries = {r['id']:r for r in old_ranking['ranked']}
            for r in result['ranked']:
                if r['id'] in old_entries:
                    assert r == old_entries[r['id']]
            rankings[weight] = result
        # Only after discovery, measurement and both orderings are complete do
        # labels determine truth availability or ranking metrics.
        truth = next(c['surfaces'][0]['text'] for c in case['input']['candidates'] if c['id'] == case['truth_id'])
        truth_ids = {c['id'] for c in expanded['candidates'] if c['surfaces'][0]['text'] == truth}
        available_before = any(c['surfaces'][0]['text'] == truth for c in base_candidates)
        available_after = bool(truth_ids)
        allowed = {c['id'] for c in expanded['expanded']['candidates'] if c['compatible']}
        row = dict(case=case['id'],status='ok',proposals=len(expanded['proposals']),added=len(expanded['added_candidate_ids']),
            compatible_added=len(expanded['compatible_added_candidate_ids']),truth_generated=any(p['text']==truth for p in expanded['proposals']),
            truth_available_before=available_before,truth_available_after=available_after,
            newly_available=available_after and not available_before,truth_compatible=bool(truth_ids&allowed),
            language_rank=rank(rankings[0]['ranked_groups'],truth_ids),combined_rank=rank(rankings[1]['ranked_groups'],truth_ids),
            previous_language_rank=rank(original['ranked_groups'],truth_ids),
            previous_combined_rank=rank(load(source/f'case-{index:03}/open-weight-1.json')['ranked_groups'],truth_ids),
            reused_scores=len(items)-len(fresh),new_scores=len(fresh))
        write(directory/'evaluation.json',row)
        rows.append(row)
        print(json.dumps(row),flush=True)
    assert all(digest(p) == sha for p,sha in frozen.items())
    assert verify_model(args.model_dir) == pin
    totals = dict(cases=len(rows),truth_available_before=sum(r['truth_available_before'] for r in rows),
        truth_available_after=sum(r['truth_available_after'] for r in rows),truth_generated=sum(r['truth_generated'] for r in rows),
        newly_available=sum(r['newly_available'] for r in rows),truth_compatible=sum(r['truth_compatible'] for r in rows),
        language_first=sum(r['language_rank']==[1,1] for r in rows),combined_first=sum(r['combined_rank']==[1,1] for r in rows),
        added_candidates=sum(r['added'] for r in rows),compatible_added=sum(r['compatible_added'] for r in rows))
    write(out/'summary.json',dict(totals=totals,rows=rows,all_frozen_hashes_verified=True,interpretation=plan['scope']))
    print(json.dumps(totals),flush=True)


if __name__ == '__main__':
    main()
