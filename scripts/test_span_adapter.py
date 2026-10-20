# Copyright (C) 2026 Toit contributors.
import copy
import json
from pathlib import Path
import tempfile
import unittest

import torch
from transformers import T5Config,T5ForConditionalGeneration
from safetensors.torch import save_file
from span_adapter import install,state,restore,load,digest


class AdapterTests(unittest.TestCase):
    def model(self):
        torch.set_num_threads(1)
        torch.manual_seed(18)
        return T5ForConditionalGeneration(T5Config(vocab_size=16,d_model=16,d_kv=4,d_ff=32,
            num_layers=1,num_decoder_layers=1,num_heads=4,dropout_rate=0,
            pad_token_id=0,eos_token_id=1,decoder_start_token_id=0)).eval()

    def data(self):
        return dict(input_ids=torch.tensor([[5,3,6,1]]),decoder_input_ids=torch.tensor([[0,3,7]]),
                    labels=torch.tensor([[-100,7,4]]),use_cache=False)

    def test_zero_adapter_is_exact_and_training_cannot_update_base(self):
        model = self.model()
        before = {k:v.clone() for k,v in model.state_dict().items()}
        expected = model(**self.data()).logits.detach()
        targets = install(model,2,4.)
        self.assertEqual(len(targets),6)
        self.assertTrue(torch.equal(expected,model(**self.data()).logits))
        trainable = [p for p in model.parameters() if p.requires_grad]
        optimizer = torch.optim.AdamW(trainable,lr=.01)
        for _ in range(2):
            optimizer.zero_grad();model(**self.data()).loss.backward();optimizer.step()
        for name,value in before.items():
            actual_name = name
            for target in targets:
                if name == target+'.weight': actual_name=target+'.base.weight'
            self.assertTrue(torch.equal(value,model.state_dict()[actual_name]),name)
        self.assertFalse(torch.equal(expected,model(**self.data()).logits))
        tensors = {k:v.clone() for k,v in state(model).items()}
        fresh = self.model();install(fresh,2,4.);restore(fresh,tensors)
        self.assertTrue(torch.equal(fresh(**self.data()).logits,model(**self.data()).logits))
        bad = copy.deepcopy(tensors);bad[next(iter(bad))][0,0]=float('nan')
        with self.assertRaises(ValueError):restore(fresh,bad)
        bad = dict(tensors);bad.pop(next(iter(bad)))
        with self.assertRaises(ValueError):restore(fresh,bad)

    def test_loader_verifies_model_hash_projection_list_and_tensors(self):
        model = self.model();targets=install(model,2,4.)
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp);weights=path/'adapter.safetensors';save_file(state(model),weights)
            manifest=dict(schema_version=1,architecture='t5-query-value-lora-v1',base_model={'pin':'test'},
                          rank=2,alpha=4.,targets=targets,weights_sha256=digest(weights),selected_step=0,dataset_sha256='data')
            (path/'manifest.json').write_text(json.dumps(manifest))
            fresh=self.model();trace=load(fresh,path,{'pin':'test'})
            self.assertEqual(trace['weights_sha256'],digest(weights))
            self.assertTrue(all(not p.requires_grad for p in fresh.parameters()))
            with self.assertRaises(ValueError):load(self.model(),path,{'pin':'wrong'})
            manifest['targets']=[];(path/'manifest.json').write_text(json.dumps(manifest))
            with self.assertRaises(ValueError):load(self.model(),path,{'pin':'test'})
            manifest['targets']=targets;manifest['weights_sha256']='wrong'
            (path/'manifest.json').write_text(json.dumps(manifest))
            with self.assertRaises(ValueError):load(self.model(),path,{'pin':'test'})


if __name__=='__main__':unittest.main()
