# Copyright (C) 2026 Toit contributors.
"""Mask already-visible ordinary words in a bundled public-document page.

This tests real raster typography/degradation with known held-out text. It does
NOT label or attempt to identify any original redaction. The four selected words
were visually checked in the original page before this experiment was prepared.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import random
import subprocess

import numpy as np
import pytesseract
from PIL import Image, ImageDraw


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    source = root / "baseline-morten/files/EFTA00037366.pdf"
    # These are ordinary visible words, not hypotheses about original redactions.
    truths = ["confirmed", "residence", "approached", "contacting"]
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    subprocess.run(["pdftoppm", "-f", "1", "-singlefile", "-r", "300", "-gray", str(source),
                    str(out / "original")], check=True, capture_output=True)
    original = Image.open(out / "original.pgm").convert("L")
    ocr = pytesseract.image_to_data(original, config="--psm 6", output_type=pytesseract.Output.DICT)
    counts = Counter(text.strip().lower() for text in ocr["text"])
    words = []
    for text in truths:
        if counts[text] != 1:
            raise ValueError(f"Expected one visible OCR occurrence of {text}: {counts[text]}")
        i = next(i for i, t in enumerate(ocr["text"]) if t.strip().lower() == text)
        if ocr["text"][i].strip() != text or float(ocr["conf"][i]) < 85:
            raise ValueError(f"OCR requires review for {text}")
        x, y, w, h = (int(ocr[key][i]) for key in ("left", "top", "width", "height"))
        crop = np.asarray(original.crop((x, y, x + w, y + h)))
        yy, xx = np.where(crop < 160)
        ink = {"x": float(x + xx.min()), "y": float(y + yy.min()),
               "w": float(xx.max() - xx.min() + 1), "h": float(yy.max() - yy.min() + 1)}
        box = {"x": ink["x"] - 2, "y": ink["y"] - 2, "w": ink["w"] + 4, "h": ink["h"] + 4}
        words.append({"text": text, "bounds": box, "ink": ink, "ocr_confidence": float(ocr["conf"][i])})
    masked = original.copy()
    draw = ImageDraw.Draw(masked)
    for word in words:
        b = word["bounds"]
        draw.rectangle((b["x"], b["y"], b["x"] + b["w"] - 1, b["y"] + b["h"] - 1), fill=0)
    path = out / "masked.pgm"
    masked.save(path)
    # Keep all targets masked in every case, so no target label can become a
    # typography calibration word in a different case on the same supplied page.
    vocabulary_source = root / "baseline-alex/unredact/data/nouns.txt"
    population = list(dict.fromkeys(t.strip() for t in vocabulary_source.read_text().splitlines()
                                   if t.strip().islower() and t.strip().isalpha() and 5 <= len(t.strip()) <= 12
                                   and t.strip() not in truths))[:10000]
    vocabulary = sorted([*random.Random(91027).sample(population, 396), *truths])
    candidates = [{"id": hashlib.sha256(text.encode()).hexdigest()[:16], "canonical_name": text,
                   "surfaces": [{"kind": "canonical", "text": text}], "source_ids": [], "context_score": None}
                  for text in vocabulary]
    page = {"id": "document-page-1", "image": "masked.pgm", "image_sha256": digest(path), "dpi": 300.,
            "redaction_regions": [word["bounds"] for word in words]}
    cases = []
    for index, word in enumerate(words):
        identity = next(c["id"] for c in candidates if c["canonical_name"] == word["text"])
        target = {"id": f"visible-word-{index}", "page": page["id"], "bounds": word["bounds"],
                  "surface_kind": "canonical", "evidence": {"kind": "width", "width": word["ink"]["w"],
                  "measurement_sigma": 1.5}, "left_context": "", "right_context": "", "dependence_key": None}
        data = {"schema_version": 1, "pages": [page], "targets": [target], "candidates": candidates}
        cases.append({"id": f"document-present-{index}", "truth_id": identity, "input": data})
        cases.append({"id": f"document-absent-{index}", "truth_id": identity,
                      "input": {**data, "candidates": [c for c in candidates if c["id"] != identity]}})
    suite = {"schema_version": 1, "track": "candidate_ranking_from_images", "cases": cases,
             "description": "Held-out visible ordinary words on the baseline repository's EFTA00037366 page; "
             "actual raster typography, synthetic masks, common known padding correction; "
             "400 candidates; original redactions untouched and unlabeled."}
    (out / "suite.json").write_text(json.dumps(suite, indent=2) + "\n")
    manifest = json.loads(args.manifest.read_text())
    # Existing generator emits absolute paths; preserve the exact tested configs.
    if any(not Path(a["executable"]).is_absolute() for a in manifest["adapters"]):
        raise ValueError("Use an absolute-path contextual-suite manifest")
    (out / "adapters.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (out / "generator-provenance.json").write_text(json.dumps({
        "source_pdf": str(source), "source_pdf_sha256": digest(source),
        "generator_sha256": digest(Path(__file__)), "dictionary_sha256": digest(vocabulary_source),
        "original_image_sha256": digest(out / "original.pgm"), "masked_image_sha256": digest(path),
        "suite_sha256": digest(out / "suite.json"), "reviewed_visible_words": words,
        "limitations": "Single page; source copied from baseline repo, not independently authenticated. "
        "Synthetic masks over visible ordinary words. No claim about original redacted identities."
    }, indent=2) + "\n")
    print(out / "suite.json")


if __name__ == "__main__":
    main()
