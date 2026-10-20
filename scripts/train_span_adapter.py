# Copyright (C) 2026 Toit contributors.
"""Adapt pinned T5 using only exported visible OCR; select on separate pixel pages."""
import argparse
import json
import os
from pathlib import Path
import random
import time

from span_proposal_provider import prepare, verify_model


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--dataset',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--model-dir',type=Path,default=Path('runs/span-model-001/t5-base'))
    p.add_argument('--steps',type=int,default=200)
    p.add_argument('--batch-size',type=int,default=4)
    p.add_argument('--validation-every',type=int,default=50)
    p.add_argument('--learning-rate',type=float,default=0.0003)
    p.add_argument('--rank',type=int,default=4)
    p.add_argument('--alpha',type=float,default=8.)
    p.add_argument('--seed',type=int,default=58431)
    p.add_argument('--threads',type=int,default=4)
    a = p.parse_args()
    if not (1 <= a.steps <= 10000 and 1 <= a.batch_size <= 32 and 1 <= a.validation_every <= a.steps
            and 0 < a.learning_rate <= 0.01 and 1 <= a.threads <= 32):
        p.error('Invalid training limits')
    a.output.mkdir(parents=True,exist_ok=False)
    os.environ['HF_HUB_OFFLINE']='1'
    os.environ['TRANSFORMERS_OFFLINE']='1'
    os.environ['TOKENIZERS_PARALLELISM']='false'
    import torch
    from transformers import T5ForConditionalGeneration,T5TokenizerFast
    from safetensors.torch import save_file,load_file
    from span_adapter import install,state,restore,digest
    torch.set_num_threads(a.threads)
    torch.manual_seed(a.seed)
    torch.use_deterministic_algorithms(True)
    rng = random.Random(a.seed)
    pin = verify_model(a.model_dir)
    tokenizer = T5TokenizerFast.from_pretrained(a.model_dir,local_files_only=True)
    model = T5ForConditionalGeneration.from_pretrained(a.model_dir,local_files_only=True,use_safetensors=True)
    targets = install(model,a.rank,a.alpha)
    dataset = json.loads(a.dataset.read_text())
    assert dataset['schema_version'] == 1
    partitions = {'train':[], 'validation':[]}
    pages = {'train':set(),'validation':set()}
    seen, rejected = set(), []
    first = tokenizer.convert_tokens_to_ids('<extra_id_0>')
    end = tokenizer.convert_tokens_to_ids('<extra_id_1>')
    for row in dataset['examples']:
        assert row['split'] in partitions and row['id'] not in seen
        seen.add(row['id'])
        pages[row['split']].add(row['origin']['pixel_sha256'])
        request = dict(schema_version=1,task='propose_context_candidates',surface_kind='canonical',
                       contexts={'span':row['context']},limits=dict(max_proposals=1,max_text_bytes=512))
        try:
            contexts,_,_ = prepare(request,tokenizer,64)
            tokens = tokenizer.encode(row['text'],add_special_tokens=False)
            if not 1 <= len(tokens) <= 64 or any(t in tokenizer.all_special_ids for t in tokens):
                raise ValueError('Unsupported target tokens')
        except ValueError as error:
            rejected.append(dict(id=row['id'],reason=str(error)))
            continue
        partitions[row['split']].append(dict(id=row['id'],encoder=contexts['span']['input_ids'],
            decoder=[tokenizer.pad_token_id,first,*tokens],labels=[-100,*tokens,end]))
    assert not pages['train'].intersection(pages['validation'])
    assert all(len(partitions[k]) >= 32 for k in partitions), 'Need at least32 examples in each split'
    trainable = [x for x in model.parameters() if x.requires_grad]
    optimizer = torch.optim.AdamW(trainable,lr=a.learning_rate,weight_decay=0.)

    def batch(rows):
        enc_width = max(len(x['encoder']) for x in rows)
        dec_width = max(len(x['decoder']) for x in rows)
        inputs = torch.full((len(rows),enc_width),tokenizer.pad_token_id,dtype=torch.long)
        attention = torch.zeros_like(inputs)
        decoder = torch.full((len(rows),dec_width),tokenizer.pad_token_id,dtype=torch.long)
        decoder_attention = torch.zeros_like(decoder)
        labels = torch.full_like(decoder,-100)
        for i,row in enumerate(rows):
            inputs[i,:len(row['encoder'])] = torch.tensor(row['encoder'])
            attention[i,:len(row['encoder'])] = 1
            decoder[i,:len(row['decoder'])] = torch.tensor(row['decoder'])
            decoder_attention[i,:len(row['decoder'])] = 1
            labels[i,:len(row['labels'])] = torch.tensor(row['labels'])
        return dict(input_ids=inputs,attention_mask=attention,decoder_input_ids=decoder,
                    decoder_attention_mask=decoder_attention,labels=labels,use_cache=False)

    def validation():
        model.eval()
        total,tokens = 0.,0
        with torch.inference_mode():
            for start in range(0,len(partitions['validation']),a.batch_size):
                data = batch(partitions['validation'][start:start+a.batch_size])
                count = int((data['labels'] != -100).sum())
                total += float(model(**data).loss)*count
                tokens += count
        return total/tokens

    frozen = {str(a.dataset):digest(a.dataset),str(Path(__file__)):digest(__file__),
              str(Path(__file__).with_name('span_adapter.py')):digest(Path(__file__).with_name('span_adapter.py')),
              str(Path(__file__).with_name('span_proposal_provider.py')):digest(Path(__file__).with_name('span_proposal_provider.py'))}
    manifest = dict(schema_version=1,architecture='t5-query-value-lora-v1',base_model=pin,
        rank=a.rank,alpha=a.alpha,targets=targets,dataset_sha256=digest(a.dataset),
        trainable_parameters=sum(x.numel() for x in trainable),
        options={k:v for k,v in vars(a).items() if not isinstance(v,Path)},
        counts={k:len(v) for k,v in partitions.items()},pages={k:sorted(v) for k,v in pages.items()},
        rejected=rejected,frozen_files=frozen,selection='Lowest validation token NLL across step0 and fixed checkpoint intervals; no test labels or test ranks.',
        interpretation='Visible OCR self-supervision on page-disjoint splits in a possibly shared document. Source OCR errors and corpus domain bias remain. Base weights frozen; only attention query/value low-rank matrices train.')
    (a.output/'plan.json').write_text(json.dumps(manifest,indent=2)+'\n')
    weights = a.output/'adapter.safetensors'
    best = validation()
    best_step = 0
    save_file(state(model),weights)
    history = [dict(step=0,validation_token_nll=best)]
    print(json.dumps(history[-1]),flush=True)
    order = []
    started = time.monotonic()
    for step in range(1,a.steps+1):
        if len(order) < a.batch_size:
            extra = list(range(len(partitions['train'])))
            rng.shuffle(extra)
            order.extend(extra)
        indices,order = order[:a.batch_size],order[a.batch_size:]
        model.train()
        optimizer.zero_grad(set_to_none=True)
        loss = model(**batch([partitions['train'][i] for i in indices])).loss
        if not torch.isfinite(loss):
            raise ValueError('Nonfinite training loss')
        loss.backward()
        torch.nn.utils.clip_grad_norm_(trainable,1.,error_if_nonfinite=True)
        optimizer.step()
        row = dict(step=step,train_token_nll=float(loss.detach()),seconds=time.monotonic()-started)
        if step % a.validation_every == 0 or step == a.steps:
            value = validation()
            row['validation_token_nll'] = value
            if value < best:
                best,best_step = value,step
                save_file(state(model),weights)
        history.append(row)
        if step % 10 == 0 or 'validation_token_nll' in row:
            print(json.dumps(row),flush=True)
            (a.output/'history.json').write_text(json.dumps(history,indent=2)+'\n')
    restore(model,load_file(weights))
    final_validation = validation()
    assert abs(final_validation-best)<1e-7
    assert all(digest(path) == sha for path,sha in frozen.items())
    assert verify_model(a.model_dir) == pin
    manifest.update(selected_step=best_step,validation_token_nll=best,base_validation_token_nll=history[0]['validation_token_nll'],
                    weights_sha256=digest(weights),base_weights_files_unchanged=True)
    (a.output/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    (a.output/'history.json').write_text(json.dumps(history,indent=2)+'\n')
    print(json.dumps(dict(selected_step=best_step,validation_token_nll=best,trainable_parameters=manifest['trainable_parameters'])),flush=True)


if __name__ == '__main__':
    main()
