# Copyright (C) 2026 Toit contributors.
"""JSON transport for visible-crop font suggestions; scores are not probabilities."""
import argparse
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image
import torch

from typography import FontEncoder, encode, normalize, prototype_arrays, prototypes, sha256


def infer(request):
    if request.get("schema_version") != 1:
        raise ValueError("schema_version must be 1")
    model_path = Path(request["model"])
    catalog = request["catalog"]
    if isinstance(catalog, str):
        catalog = json.loads(Path(catalog).read_text())
    if not isinstance(catalog, list) or not 1 <= len(catalog) <= 1000:
        raise ValueError("catalog must contain 1..1000 font entries")
    ids = [font["id"] for font in catalog]
    if len(set(ids)) != len(ids):
        raise ValueError("catalog IDs must be unique")
    for font in catalog:
        actual = sha256(font["path"])
        if actual != font["sha256"]:
            raise ValueError(f"font hash mismatch: {font['id']}")
    images = request["images"]
    if not isinstance(images, list) or not 1 <= len(images) <= 128:
        raise ValueError("images must contain 1..128 visible-text crops")
    arrays, evidence = [], []
    for item in images:
        item = {"path": item} if isinstance(item, str) else item
        with Image.open(item["path"]) as original:
            original.load()
            image = original.convert("L")
            if "box" in item:
                box = item["box"]
                if (len(box) != 4 or not all(isinstance(v, int) for v in box)
                        or not 0 <= box[0] < box[2] <= image.width
                        or not 0 <= box[1] < box[3] <= image.height):
                    raise ValueError("box must be integer [left, top, right, bottom] within image")
                image = image.crop(tuple(box))
            arrays.append(normalize(image))
        evidence.append(dict(path=str(item["path"]), sha256=sha256(item["path"]), box=item.get("box")))
    checkpoint = torch.load(model_path, map_location="cpu", weights_only=True)
    if checkpoint["architecture"] != "font-cnn-v1":
        raise ValueError("unsupported model architecture")
    model = FontEncoder(checkpoint["classes"])
    model.load_state_dict(checkpoint["state_dict"])
    query = encode(model, arrays)
    reference = prototypes(encode(model, prototype_arrays(catalog)), len(catalog))
    per_crop = query @ reference.T
    scores = per_crop.mean(0)
    top_k = request.get("top_k", 10)
    if not isinstance(top_k, int) or not 1 <= top_k <= 1000:
        raise ValueError("top_k must be an integer in 1..1000")
    ranking = np.argsort(-scores, kind="stable")[:top_k]
    return dict(schema_version=1, status="ok", model_sha256=sha256(model_path),
                architecture=checkpoint["architecture"], evidence=evidence,
                score_semantics="mean cosine similarity across visible crops; not a calibrated probability",
                catalog_size=len(catalog), suggestions=[dict(
                    id=catalog[i]["id"], path=catalog[i]["path"], sha256=catalog[i]["sha256"],
                    score=float(scores[i]), per_crop_scores=per_crop[:, i].tolist()) for i in ranking],
                warnings=["All supplied crops should share typography; heterogeneous styles can dilute ranking",
                          "Synthetic-renderer training; real-document accuracy has not been established",
                          "Verify suggested fonts with the physical renderer before excluding candidates"])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, help="JSON request; defaults to stdin")
    parser.add_argument("--threads", type=int, default=4)
    args = parser.parse_args()
    torch.set_num_threads(args.threads)
    try:
        request = json.loads(args.input.read_text() if args.input else sys.stdin.read())
        response = infer(request)
    except (KeyError, ValueError, OSError, RuntimeError) as error:
        print(json.dumps(dict(schema_version=1, status="error", error=str(error))))
        sys.exit(1)
    print(json.dumps(response, allow_nan=False))


if __name__ == "__main__":
    main()
