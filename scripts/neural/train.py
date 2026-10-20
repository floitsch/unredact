# Copyright (C) 2026 Toit contributors.
"""Train on synthetic crops; evaluate disjoint text, font families, and degradation."""
import argparse
import json
import platform
import random
import time
from pathlib import Path

import numpy as np
import PIL
import torch
from torch.nn import functional as F

from typography import (FontEncoder, PROTOTYPE_TEXTS, baseline, encode,
                        prototype_arrays, prototypes, render, sha256)

TRAIN_WORDS = "analysis report several printed documents useful evidence consistent spacing regular character paragraph observation original reference measure project method compare process render detailed establish available public letter group design result system pattern sample model precise estimate typical common identify width paper black white margin reading display image source configuration".split()
VALID_WORDS = "checking stable proposal winter garden river second twelve examine supplied version correct statement training unknown outside narrow browser scanning department theory handle book chapter stone valley".split()
TEST_WORDS = "independent evaluation withheld experiment governance uncertainty alphabet alignment distortion reproduction contrast architecture inspection revision committee executive summary intelligence procedure qualification preservation affirmative investigate jurisdiction explanation announcement".split()


def catalog():
    fonts = []
    specs = [("liberation/LiberationSerif", "regular", "train"),
             ("liberation/LiberationSans", "regular", "train"),
             ("TTF/DejaVuSerif", "regular", "train"),
             ("TTF/DejaVuSans", "oblique", "train"),
             ("lato/Lato", "regular", "train"),
             ("TTF/Roboto", "regular", "train"),
             ("TTF/DejaVuSansMono", "oblique", "unseen_family"),
             ("noto/NotoSans", "regular", "unseen_family"),
             ("TTF/Merriweather", "regular", "unseen_family")]
    for base, style_kind, split in specs:
        for style in ["Regular", "Bold", "Oblique" if style_kind == "oblique" else "Italic",
                      "BoldOblique" if style_kind == "oblique" else "BoldItalic"]:
            name = base + ("" if style == "Regular" and "DejaVu" in base else "-"+style) + ".ttf"
            path = Path("/usr/share/fonts") / name
            if not path.exists():
                raise FileNotFoundError(path)
            fonts.append(dict(id=path.stem, family=base.split("/")[-1], style=style,
                              path=str(path), sha256=sha256(path), split=split))
    return fonts


def make_data(fonts, words, samples, seed, severe=False):
    rng = np.random.default_rng(seed)
    arrays, labels, texts = [], [], []
    for label, font in enumerate(fonts):
        for _ in range(samples):
            text = " ".join(rng.choice(words, size=3, replace=False))
            arrays.append(render(font["path"], text, rng, severe))
            labels.append(label)
            texts.append(text)
    return np.stack(arrays), np.array(labels), texts


def metrics(features, proto, labels, catalog, indices=None):
    indices = np.arange(len(labels)) if indices is None else indices
    scores = features[indices] @ proto.T
    ranks = np.argsort(-scores, axis=1, kind="stable")
    truth = labels[indices]
    return dict(count=len(indices), top1=float(np.mean(ranks[:, 0] == truth)),
                top3=float(np.mean(np.any(ranks[:, :3] == truth[:, None], axis=1))),
                top5=float(np.mean(np.any(ranks[:, :5] == truth[:, None], axis=1))),
                family_top1=float(np.mean([catalog[p]["family"] == catalog[t]["family"]
                                         for p, t in zip(ranks[:, 0], truth)])))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--epochs", type=int, default=24)
    parser.add_argument("--samples", type=int, default=160)
    parser.add_argument("--threads", type=int, default=8)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    torch.set_num_threads(args.threads)
    torch.manual_seed(29181)
    random.seed(29181)
    np.random.seed(29181)
    torch.use_deterministic_algorithms(True)
    fonts = catalog()
    seen = [x for x in fonts if x["split"] == "train"]
    assert not (set(TRAIN_WORDS)&set(TEST_WORDS) or set(TRAIN_WORDS)&set(VALID_WORDS) or set(VALID_WORDS)&set(TEST_WORDS))
    manifest = dict(seed=29181, fonts=fonts, train_words=TRAIN_WORDS, validation_words=VALID_WORDS,
                    test_words=TEST_WORDS, prototype_texts=PROTOTYPE_TEXTS,
                    epochs=args.epochs, samples_per_train_font=args.samples,
                    environment=dict(python=platform.python_version(), torch=torch.__version__,
                                     pillow=PIL.__version__, numpy=np.__version__),
                    limitations=["Synthetic Pillow renderer only; no real scan accuracy established",
                                 "Unseen families remain part of the inference candidate catalog",
                                 "Same font binaries and renderer generate gallery and queries",
                                 "Scores are cosine similarities, not identity probabilities"])
    (args.output/"manifest.json").write_text(json.dumps(manifest, indent=2))
    (args.output/"catalog.json").write_text(json.dumps(fonts, indent=2))
    print("Generating training and validation crops", flush=True)
    train_x, train_y, train_text = make_data(seen, TRAIN_WORDS, args.samples, 111)
    valid_x, valid_y, valid_text = make_data(seen, VALID_WORDS, 24, 222)
    np.savez_compressed(args.output/"training-data.npz", images=train_x, labels=train_y,
                        texts=np.array(train_text), validation_images=valid_x,
                        validation_labels=valid_y, validation_texts=np.array(valid_text))
    model = FontEncoder(len(seen))
    optimizer = torch.optim.AdamW(model.parameters(), lr=.002, weight_decay=.0001)
    gallery = prototype_arrays(fonts)
    history = []
    best = -1
    started = time.time()
    for epoch in range(args.epochs):
        model.train()
        losses = []
        permutation = np.random.default_rng(1000+epoch).permutation(len(train_x))
        for start in range(0, len(permutation), 64):
            ids = permutation[start:start+64]
            x = torch.from_numpy(train_x[ids]).float().unsqueeze(1)/255
            y = torch.from_numpy(train_y[ids])
            embedding = model(x)
            # Normalized classifier encourages an embedding usable for unseen-font retrieval.
            logits = 16 * embedding @ F.normalize(model.classifier.weight, dim=1).T
            loss = F.cross_entropy(logits, y)
            optimizer.zero_grad()
            loss.backward()
            optimizer.step()
            losses.append(float(loss.detach()))
        proto = prototypes(encode(model, gallery), len(fonts))
        valid = metrics(encode(model, valid_x), proto, valid_y, fonts)
        row = dict(epoch=epoch+1, loss=float(np.mean(losses)), validation=valid,
                   elapsed_seconds=time.time()-started)
        history.append(row)
        print(json.dumps(row), flush=True)
        if valid["top1"] > best:
            best = valid["top1"]
            torch.save(dict(state_dict=model.state_dict(), classes=len(seen), epoch=epoch+1,
                            architecture="font-cnn-v1", manifest_sha256=sha256(args.output/"manifest.json")),
                       args.output/"model.pt")
        (args.output/"history.json").write_text(json.dumps(history, indent=2))
    checkpoint = torch.load(args.output/"model.pt", weights_only=True)
    model.load_state_dict(checkpoint["state_dict"])
    proto = prototypes(encode(model, gallery), len(fonts))
    base_proto = prototypes(baseline(gallery), len(fonts))
    report = dict(selected_epoch=checkpoint["epoch"], model_sha256=sha256(args.output/"model.pt"),
                  parameters=sum(p.numel() for p in model.parameters()), evaluations={})
    for condition, severe in [("standard", False), ("shifted_degradation", True)]:
        x, y, texts = make_data(fonts, TEST_WORDS, 24, 333 if not severe else 444, severe)
        features, base_features = encode(model, x), baseline(x)
        np.savez_compressed(args.output/f"evaluation-{condition}.npz", images=x, labels=y,
                            texts=np.array(texts), neural_scores=features@proto.T,
                            baseline_scores=base_features@base_proto.T)
        for split in ["train", "unseen_family"]:
            indices = np.array([i for i, label in enumerate(y) if fonts[label]["split"] == split])
            report["evaluations"][condition+"/"+split] = dict(
                neural=metrics(features, proto, y, fonts, indices),
                baseline=metrics(base_features, base_proto, y, fonts, indices))
    np.savez_compressed(args.output/"gallery.npz", embeddings=proto, baseline=base_proto)
    (args.output/"evaluation.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2), flush=True)


if __name__ == "__main__":
    main()
