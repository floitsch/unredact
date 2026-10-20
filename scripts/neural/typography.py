# Copyright (C) 2026 Toit contributors.
"""Small visible-text font embedding and reproducible scan simulator."""
from __future__ import annotations

import hashlib
import io
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont
import torch
from torch import nn
from torch.nn import functional as F

HEIGHT, WIDTH = 48, 256
PROTOTYPE_TEXTS = [
    "Typography requires careful measurement", "Visible letters preserve useful evidence",
    "Several independent observations agree", "Document analysis compares original shapes",
    "Fractional positions affect narrow strokes", "Public records include different typefaces",
    "Printed paragraphs contain repeated pairs", "Calibration estimates uncertain spacing",
]


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def normalize(image):
    """Tight crop and normalize height, preserving aspect ratio; center crop long lines."""
    array = np.asarray(image.convert("L"))
    ys, xs = np.where(array < 180)
    if len(xs) < 5:
        raise ValueError("crop contains insufficient dark text")
    crop = Image.fromarray(array).crop((max(0, xs.min()-1), max(0, ys.min()-1),
                                       min(image.width, xs.max()+2), min(image.height, ys.max()+2)))
    w = max(1, round(crop.width * (HEIGHT-8) / crop.height))
    crop = crop.resize((w, HEIGHT-8), Image.Resampling.LANCZOS)
    out = Image.new("L", (WIDTH, HEIGHT), 255)
    out.paste(crop, ((WIDTH-w)//2, 4))
    return (255 - np.asarray(out)).astype(np.uint8)


def render(path, text, rng=None, severe=False):
    rng = rng or np.random.default_rng(0)
    size = int(rng.integers(25, 45)) if severe else int(rng.integers(30, 53))
    font = ImageFont.truetype(str(path), size)
    box = font.getbbox(text)
    image = Image.new("L", (box[2]-box[0]+24, box[3]-box[1]+24), 255)
    ImageDraw.Draw(image).text((12-box[0], 12-box[1]), text, font=font, fill=0)
    stretch = rng.uniform(.90, 1.10) if not severe else rng.uniform(.78, 1.22)
    image = image.resize((max(1, round(image.width*stretch)), image.height), Image.Resampling.BICUBIC)
    image = image.rotate(float(rng.uniform(-.6, .6) if not severe else rng.uniform(-1.4, 1.4)),
                         resample=Image.Resampling.BICUBIC, expand=True, fillcolor=255)
    image = image.filter(ImageFilter.GaussianBlur(float(rng.uniform(0, .65) if not severe else rng.uniform(.8, 1.3))))
    array = np.asarray(image).astype(float)
    array += rng.normal(0, 2.0 if not severe else 7.0, array.shape)
    image = Image.fromarray(np.uint8(np.clip(array, 0, 255)))
    if rng.random() < .45:
        image = image.point(lambda p: 255 if p > 155 else 0)
    else:
        stream = io.BytesIO()
        image.save(stream, format="JPEG", quality=int(rng.integers(60, 96) if not severe else rng.integers(30, 60)))
        image = Image.open(io.BytesIO(stream.getvalue())).convert("L")
    return normalize(image)


class FontEncoder(nn.Module):
    def __init__(self, classes=24):
        super().__init__()
        layers = []
        previous = 1
        for channels in (16, 32, 64, 96):
            layers.extend([nn.Conv2d(previous, channels, 3, padding=1),
                           nn.BatchNorm2d(channels), nn.SiLU(), nn.MaxPool2d(2)])
            previous = channels
        self.features = nn.Sequential(*layers)
        self.project = nn.Linear(192, 64)
        self.classifier = nn.Linear(64, classes)

    def forward(self, x):
        feature = self.features(x)
        flat = feature.flatten(2)
        embedding = self.project(torch.cat([flat.mean(2), flat.std(2, correction=0)], 1))
        return F.normalize(embedding, dim=1)


def encode(model, arrays, batch=128):
    model.eval()
    outputs = []
    with torch.inference_mode():
        for start in range(0, len(arrays), batch):
            x = torch.from_numpy(np.array(arrays[start:start+batch])).float().unsqueeze(1)/255
            outputs.append(model(x).numpy())
    return np.concatenate(outputs)


def baseline(arrays):
    """Non-neural horizontal/vertical stroke transition and density descriptor."""
    a = np.asarray(arrays, dtype=np.float32)/255
    ink = a.mean(2)
    vertical = np.abs(np.diff(a, axis=1)).mean(2)
    horizontal = np.abs(np.diff(a, axis=2)).mean(2)
    result = np.concatenate([ink, vertical, horizontal], axis=1)
    return result / np.maximum(np.linalg.norm(result, axis=1, keepdims=True), 1e-8)


def prototype_arrays(catalog):
    arrays = []
    for font in catalog:
        for index, text in enumerate(PROTOTYPE_TEXTS):
            arrays.append(render(font["path"], text, np.random.default_rng(8100+index)))
    return arrays


def prototypes(features, count):
    result = features.reshape(count, len(PROTOTYPE_TEXTS), -1).mean(1)
    return result / np.maximum(np.linalg.norm(result, axis=1, keepdims=True), 1e-8)
