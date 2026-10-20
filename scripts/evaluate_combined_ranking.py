# Copyright (C) 2026 Toit contributors.
"""Evaluate a fixed combined score on new masked locations and common T5 scores.

An explicitly ungraded protocol-capture pass obtains Rust-selected context.
Full-vocabulary closed scores support equal-language external comparisons.
Open candidates are harvested without the original test pages. Final rankings
run through the Rust CLI with strictly checked cached model scores.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

from evaluate_common_language import truth_rank
from evaluate_span_ranking import rank
from span_context_provider import prepare, score_items, verify_model


def load(path):
    return json.loads(Path(path).read_text())


def write(path, data):
    Path(path).write_text(json.dumps(data, indent=2) + '\n')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--capture', type=Path)
    parser.add_argument('--model-dir', type=Path, default=Path('runs/span-model-001/t5-base'))
    args = parser.parse_args()
    if args.capture:
        request = json.load(sys.stdin)
        assert not args.capture.exists(), 'Capture expects a single small request'
        write(args.capture, request)
        print(json.dumps(dict(schema_version=1, scores=[dict(id=i['id'],target=i['target'],
            token_count=1,token_log_score_sum=0.) for i in request['items']],
            method='PROTOCOL_CAPTURE_ONLY_NOT_MODEL_SCORES')))
        return
    if not args.root or not args.output:
        parser.error('Supply --root and --output')
    root, out = args.root, args.output
    out.mkdir(parents=True, exist_ok=False)
    plan = load(root/'plan.json')
    assert plan['physical_weight'] == 1.0 and plan['control_weight'] == 0.0
    original_freeze = load(root/'frozen.json')['files']
    assert all(digest(p) == sha for p,sha in original_freeze.items())
    suite = load(root/'suite.json')
    comparison = load(root/'comparison/comparison.json')
    binary = Path('target/release/unredact').resolve()
    replay = Path('scripts/evaluate_bounded_controls.py').resolve()
    replay_python = Path('runs/baseline-venv/bin/python').absolute()
    artifacts = [Path(__file__),replay,root/'comparison/comparison.json',root/'corpus-projects.json']
    for path in (root/'comparison').glob('*/work/project/project.json'):
        artifacts += [path,path.parent.parent/'candidates.json',path.parent.parent/'native-result.json']
    for directory in load(root/'corpus-projects.json'):
        artifacts.append(Path(directory)/'project.json')
    artifacts += [*Path('scripts').glob('*span*provider.py'),Path('scripts/evaluate_common_language.py')]
    frozen = {**original_freeze, **{str(p):digest(p) for p in artifacts}}
    provenance = verify_model(args.model_dir)
    write(out/'frozen.json',dict(files=frozen,model=provenance,weight=1.0,
        interpretation='Fixed weight1 vs0. Reused document, new masked locations. Closed300 includes truth; open corpus excludes every original test page.'))

    def run(command, error_path, timeout=330):
        proc = subprocess.run(list(map(str,command)),capture_output=True,text=True,timeout=timeout)
        error_path.write_text(proc.stderr)
        if proc.returncode:
            raise RuntimeError(proc.stderr)
        return proc.stdout

    open_path = out/'open-candidates.json'
    command = [binary,'harvest','--max-candidates','50000','--output',open_path]
    for directory in load(root/'corpus-projects.json'):
        command += ['--project',directory]
    run(command,out/'harvest.stderr')
    vocabulary = load(open_path)['candidates']
    open_digest = digest(open_path)
    write(out/'corpus-summary.json',dict(sha256=open_digest,counts=load(open_path)['counts']))
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    os.environ['TOKENIZERS_PARALLELISM'] = 'false'
    import torch
    from transformers import T5TokenizerFast, T5ForConditionalGeneration
    torch.set_num_threads(4)
    tokenizer = T5TokenizerFast.from_pretrained(args.model_dir,local_files_only=True)
    model = T5ForConditionalGeneration.from_pretrained(args.model_dir,local_files_only=True,use_safetensors=True).eval()
    rows = []
    for index, case in enumerate(suite['cases']):
        directory = out/f'case-{index:03}'
        directory.mkdir()
        entries = [e for e in comparison['executions'] if e['case'] == case['id']]
        local = next(e for e in entries if e['adapter'] == 'width-fringe-default-filtered')
        row = dict(case=case['id'],tracks={})
        if local['error'] or local['result']['status'] != 'ok':
            row['error'] = local.get('error') or local['result'].get('reason')
            rows.append(row);write(directory/'evaluation.json',row);continue
        base = root/'comparison'/f'case-{index:03}-repeat-00-tool-00'/'work'
        project = base/'project'
        closed_path = base/'candidates.json'
        capture = directory/'protocol-request.json'
        common = [binary,'context-rank','--project',project,'--group','comparison',
                  '--paragraph-context','--sequence-score']
        run(common+['--candidates',closed_path,'--provider',replay_python,
            '--provider-arg',Path(__file__).resolve(),'--provider-arg=--capture',
            '--provider-arg',capture.resolve(),'--output',directory/'UNSCORED-protocol-capture.json'],
            directory/'capture.stderr')
        contexts = load(capture)['contexts']
        assert len(contexts) == 1
        target = next(iter(contexts))
        for track, candidate_path in [('closed',closed_path),('open',open_path)]:
            candidates = load(candidate_path)['candidates']
            physical_path = directory/f'{track}-physical.json'
            run([binary,'solve','--project',project,'--group','comparison','--candidates',candidate_path,
                 '--limit',len(candidates),'--output',physical_path],directory/f'{track}-solve.stderr')
            physical = load(physical_path)
            allowed = {c['id'] for c in physical['candidates'] if c['compatible']}
            # The closed common-language control scores every candidate, including
            # ones retained only by an external filter. Open scoring is bounded
            # to the local compatible set as in ordinary CLI operation.
            items = [dict(id=c['id'],target=target,text=c['surfaces'][0]['text'])
                     for c in candidates if track == 'closed' or c['id'] in allowed]
            scores, diagnostics = [], []
            for start in range(0,len(items),2048):
                request = dict(schema_version=1,task='score_context_candidates',contexts=contexts,items=items[start:start+2048])
                prepared, used = prepare(request,tokenizer,64)
                stats = score_items(prepared,used,tokenizer,model,torch,16)
                scores.extend(prepared)
                diagnostics.append(dict(contexts_used=used,forward_states=stats))
            cached = directory/f'{track}-scores.json'
            write(cached,dict(request=dict(schema_version=1,task='score_context_candidates',contexts=contexts,items=items),
                              scores=scores,diagnostics=diagnostics,model=provenance))
            reports = {}
            for weight in [0,1]:
                path = directory/f'{track}-weight-{weight}.json'
                run(common+['--candidates',candidate_path,'--physical-weight',weight,
                    '--provider',replay_python,'--provider-arg',replay,'--provider-arg=--replay',
                    '--provider-arg',cached.resolve(),'--output',path],directory/f'{track}-weight-{weight}.stderr')
                report = load(path)
                assert report['physical'] == physical and report['contexts'] == contexts
                reports[weight] = report
            # Grade after all scores and both Rust rankings have been produced.
            truth_text = next(c['surfaces'][0]['text'] for c in case['input']['candidates'] if c['id'] == case['truth_id'])
            truth_ids = {c['id'] for c in candidates if c['surfaces'][0]['text'] == truth_text}
            ranked = {w:{c['id']:c for c in r['ranked']} for w,r in reports.items()}
            assert ranked[0].keys() == ranked[1].keys() == allowed
            physical_by_id = {c['id']:c for c in physical['candidates']}
            for identity in allowed:
                a,b = ranked[0][identity],ranked[1][identity]
                assert a['observations'] == b['observations']
                assert a['mean_sequence_log_score'] == b['mean_sequence_log_score']
                assert b['combined_score'] == a['mean_sequence_log_score'] - physical_by_id[identity]['evidence_score']
            result = dict(status='ok',candidates=len(candidates),compatible=len(allowed),truth_available=bool(truth_ids),
                          truth_compatible=bool(allowed & truth_ids),
                          language_rank=rank(reports[0]['ranked_groups'],truth_ids),
                          combined_rank=rank(reports[1]['ranked_groups'],truth_ids))
            if track == 'closed':
                by_id = {s['id']:s['token_log_score_sum'] for s in scores}
                result['common_language'] = {'language_only':dict(status='ok',truth_rank=truth_rank(by_id,case['truth_id']))}
                for external in entries:
                    if external['adapter'] == local['adapter']:
                        continue
                    assert external['input_sha256'] == local['input_sha256']
                    if external['error'] or external['result']['status'] != 'ok':
                        result['common_language'][external['adapter']] = dict(status='error_or_unsupported',reason=external['error'] or external['result'].get('reason'))
                    elif external['result']['compatible'] is None:
                        result['common_language'][external['adapter']] = dict(status='not_available',reason='Adapter exposes ranking but no compatible set; no new cutoff is invented.')
                    else:
                        ids = set(external['result']['compatible'])
                        result['common_language'][external['adapter']] = dict(status='ok',compatible=len(ids),
                            truth_compatible=case['truth_id'] in ids,truth_rank=truth_rank({k:by_id[k] for k in ids},case['truth_id']))
            row['tracks'][track] = result
            write(directory/'evaluation.json',row)
            print(case['id'],track,json.dumps(result),flush=True)
        rows.append(row)
    assert digest(open_path) == open_digest
    assert all(digest(p) == sha for p,sha in frozen.items())
    assert verify_model(args.model_dir) == provenance
    totals = {}
    for track in ['closed','open']:
        valid = [row['tracks'][track] for row in rows if track in row['tracks']]
        totals[track] = dict(cases=len(rows),successful=len(valid),truth_available=sum(v['truth_available'] for v in valid),
            truth_compatible=sum(v['truth_compatible'] for v in valid),language_first=sum(v['language_rank']==[1,1] for v in valid),
            combined_first=sum(v['combined_rank']==[1,1] for v in valid))
    write(out/'summary.json',dict(rows=rows,totals=totals,all_frozen_hashes_verified=True,
        interpretation='Fixed weight1 on eight new masked locations from a reused document, including a word fragment. Seven complete words and one fragment; two masks per page. Closed vocabulary includes truths. Open vocabulary excludes original test pages. Common-language comparisons adapt geometric filters, not upstream AI.'))
    print(json.dumps(totals),flush=True)


if __name__ == '__main__':
    main()
