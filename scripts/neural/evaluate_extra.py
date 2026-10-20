# Copyright (C) 2026 Toit contributors.
"""Frozen-model independent renderer, random encoder, and pooled-crop controls."""
import argparse
import json
import subprocess
from pathlib import Path

import numpy as np
from PIL import Image
import torch

from train import TEST_WORDS, metrics
from typography import (FontEncoder, baseline, encode, normalize,
                        prototype_arrays, prototypes, sha256)


def from_scores(scores, labels, catalog, split):
    indices = np.array([i for i, label in enumerate(labels) if catalog[label]["split"] == split])
    return metrics(scores, np.eye(len(catalog)), labels, catalog, indices)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--threads", type=int, default=4)
    args = parser.parse_args()
    torch.set_num_threads(args.threads)
    root = args.run
    fonts = json.loads((root/"catalog.json").read_text())
    checkpoint = torch.load(root/"model.pt", weights_only=True)
    model = FontEncoder(checkpoint["classes"])
    model.load_state_dict(checkpoint["state_dict"])
    torch.manual_seed(73001)
    random_model = FontEncoder(checkpoint["classes"])
    gallery = prototype_arrays(fonts)
    proto = prototypes(encode(model, gallery), len(fonts))
    random_proto = prototypes(encode(random_model, gallery), len(fonts))
    base_proto = prototypes(baseline(gallery), len(fonts))
    report = dict(model_sha256=sha256(root/"model.pt"), pooled={}, random_encoder={}, cairo={})
    for condition in ["standard", "shifted_degradation"]:
        data = np.load(root/f"evaluation-{condition}.npz")
        random_scores = encode(random_model, data["images"]) @ random_proto.T
        for split in ["train", "unseen_family"]:
            report["random_encoder"][condition+"/"+split] = from_scores(random_scores, data["labels"], fonts, split)
            for method in ["neural", "baseline"]:
                pooled = data[method+"_scores"].reshape(len(fonts), 4, 6, len(fonts)).mean(2).reshape(-1, len(fonts))
                labels = np.repeat(np.arange(len(fonts)), 4)
                report["pooled"][condition+"/"+split+"/"+method] = from_scores(pooled, labels, fonts, split)
    renderer = root/"render-cairo"
    source = Path(__file__).with_name("render_cairo.c")
    flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "cairo", "freetype2"], text=True).split()
    subprocess.run(["cc", "-O2", "-Wall", "-Wextra", "-Werror", str(source), "-o", str(renderer), *flags, "-lm"], check=True)
    report["cairo_provenance"] = dict(source_sha256=sha256(source), binary_sha256=sha256(renderer),
        versions=subprocess.check_output(["pkg-config", "--modversion", "cairo", "freetype2"], text=True).splitlines(),
        scope="Cairo show_text, exact font files, no shaping beyond Cairo toy text API; clean raster images")
    for hinted in [False, True]:
        rng = np.random.default_rng(55301)
        arrays, labels, texts = [], [], []
        for label, font in enumerate(fonts):
            for index in range(12):
                text = " ".join(rng.choice(TEST_WORDS, 3, replace=False))
                subprocess.run([str(renderer), font["path"], text, str(28+index*2),
                                str(root/"cairo-temporary.png"), str(int(hinted))], check=True)
                with Image.open(root/"cairo-temporary.png") as image:
                    arrays.append(normalize(image))
                labels.append(label)
                texts.append(text)
        arrays, labels = np.stack(arrays), np.array(labels)
        scores = encode(model, arrays) @ proto.T
        base_scores = baseline(arrays) @ base_proto.T
        random_scores = encode(random_model, arrays) @ random_proto.T
        for split in ["train", "unseen_family"]:
            report["cairo"][f"hinted={hinted}/{split}"] = dict(
                neural=from_scores(scores, labels, fonts, split),
                baseline=from_scores(base_scores, labels, fonts, split),
                random_encoder=from_scores(random_scores, labels, fonts, split))
        np.savez_compressed(root/f"evaluation-cairo-hinted-{hinted}.npz", images=arrays,
                            labels=labels, texts=np.array(texts), neural_scores=scores,
                            baseline_scores=base_scores, random_scores=random_scores)
    (root/"cairo-temporary.png").unlink()
    (root/"evaluation-extra.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
