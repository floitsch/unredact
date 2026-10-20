# Copyright (C) 2026 Toit contributors.
"""Small optional query/value adapters for the pinned offline T5 model."""
import hashlib
import json
import math
from pathlib import Path

import torch
from torch import nn


class LowRankLinear(nn.Module):
    def __init__(self, base, rank, alpha):
        super().__init__()
        self.base = base
        self.scale = alpha / rank
        self.a = nn.Parameter(torch.empty(rank, base.in_features))
        self.b = nn.Parameter(torch.zeros(base.out_features, rank))
        nn.init.kaiming_uniform_(self.a, a=math.sqrt(5))

    def forward(self, x):
        return self.base(x) + (x @ self.a.T @ self.b.T) * self.scale


def install(model, rank=4, alpha=8.):
    from transformers.models.t5.modeling_t5 import T5Attention
    if type(rank) is not int or not 1 <= rank <= 64 or not math.isfinite(alpha) or not 0 < alpha <= 128:
        raise ValueError('Invalid adapter rank or alpha')
    for parameter in model.parameters():
        parameter.requires_grad_(False)
    targets = []
    for name, module in list(model.named_modules()):
        if not isinstance(module,T5Attention):
            continue
        for leaf in ['q','v']:
            base = getattr(module,leaf)
            if not isinstance(base,nn.Linear) or base.bias is not None:
                raise ValueError('Expected unmodified bias-free T5 attention projection')
            setattr(module,leaf,LowRankLinear(base,rank,alpha))
            targets.append(name+'.'+leaf)
    if not targets:
        raise ValueError('No supported attention projections found')
    return targets


def state(model):
    return {name+'.'+leaf: getattr(module,leaf).detach().cpu().contiguous()
            for name,module in model.named_modules() if isinstance(module,LowRankLinear)
            for leaf in ['a','b']}


def restore(model, tensors):
    expected = state(model)
    if tensors.keys() != expected.keys():
        raise ValueError('Adapter tensor keys do not match the model')
    for name,tensor in tensors.items():
        if tensor.dtype != torch.float32 or tensor.shape != expected[name].shape or not torch.isfinite(tensor).all():
            raise ValueError('Invalid adapter tensor shape, type or values: '+name)
    modules = dict(model.named_modules())
    with torch.no_grad():
        for name,tensor in tensors.items():
            parent,leaf = name.rsplit('.',1)
            getattr(modules[parent],leaf).copy_(tensor)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load(model, directory, base_pin):
    from safetensors.torch import load as load_tensors
    directory = Path(directory)
    manifest_path, weights = directory/'manifest.json', directory/'adapter.safetensors'
    if manifest_path.stat().st_size > 1024*1024 or weights.stat().st_size > 128*1024*1024:
        raise ValueError('Adapter artifact too large')
    manifest_bytes = manifest_path.read_bytes()
    weights_bytes = weights.read_bytes()
    manifest = json.loads(manifest_bytes)
    if manifest['schema_version'] != 1 or manifest['architecture'] != 't5-query-value-lora-v1':
        raise ValueError('Unsupported adapter format')
    if manifest['base_model'] != base_pin or hashlib.sha256(weights_bytes).hexdigest() != manifest['weights_sha256']:
        raise ValueError('Adapter base-model or weights hash mismatch')
    targets = install(model,manifest['rank'],manifest['alpha'])
    if targets != manifest['targets']:
        raise ValueError('Adapter projection list differs from the model')
    restore(model,load_tensors(weights_bytes))
    for parameter in model.parameters():
        parameter.requires_grad_(False)
    model.eval()
    return dict(manifest_sha256=hashlib.sha256(manifest_bytes).hexdigest(),weights_sha256=manifest['weights_sha256'],
                architecture=manifest['architecture'],rank=manifest['rank'],alpha=manifest['alpha'],
                selected_step=manifest['selected_step'],dataset_sha256=manifest['dataset_sha256'],
                source_sha256=digest(__file__))
