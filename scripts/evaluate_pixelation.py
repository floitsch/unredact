# Copyright (C) 2026 Toit contributors.
"""Independent Pillow/NumPy mosaic fixtures and negative controls.

Only the image and a fixed grid search range reach the application. Ground-truth
text, block size, phase, and degradation labels remain in this controller.
This evaluates grid detection, not hidden-text recovery or model probabilities.
"""
import argparse
from collections import defaultdict
import hashlib
import io
import json
from pathlib import Path
import subprocess

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont, __version__ as pillow_version


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mosaic(image, block, px, py):
    source = np.asarray(image)
    result = np.empty_like(source)
    height, width = source.shape
    for y in range(-py, height, block):
        for x in range(-px, width, block):
            area = np.s_[max(y, 0):min(y + block, height),
                         max(x, 0):min(x + block, width)]
            result[area] = int(source[area].mean())
    return Image.fromarray(result)


def jpeg(image, quality):
    stream = io.BytesIO()
    image.save(stream, format="JPEG", quality=quality)
    stream.seek(0)
    return Image.open(stream).convert("L")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/unredact"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Use a fresh output directory")
    out = args.output.resolve()
    out.mkdir(parents=True)
    binary = args.binary.resolve()

    def run(*argv):
        return subprocess.check_output([str(binary), *map(str, argv)], text=True)

    fonts = [Path(p) for p in run("fonts").splitlines()[:3]]
    if len(fonts) != 3:
        raise RuntimeError("Three installed fonts required")
    rng = np.random.default_rng(20261019)
    rows = []

    def evaluate(image, family, label, expected=None, **metadata):
        number = len(rows)
        path = out / f"image-{number:03}.pgm"
        report_path = out / f"analysis-{number:03}.json"
        image.save(path)
        run("analyze-pixels", "--image", path, "--min-block", 2,
            "--max-block", 16, "--keep", 3, "--output", report_path)
        report = json.loads(report_path.read_text())
        grids = [h["grid"] for h in report["hypotheses"]]
        rows.append({"image": path.name, "image_sha256": digest(path),
                     "analysis": report_path.name, "family": family, "label": label,
                     "expected_grid": expected, "detected": bool(grids),
                     "exact_first": expected == grids[0] if grids and expected else False,
                     "exact_retained": expected in grids if expected else False,
                     "status": report["status"], "grids": grids, **metadata})

    for font_index, font_path in enumerate(fonts):
        for size in (26, 44):
            raw = Image.new("L", (760, 110), 255)
            text = ("AVATAR office typography", "Amber Vale and Rowan West",
                    "Measurements require evidence")[font_index]
            ImageDraw.Draw(raw).text((18.25, 20.5), text,
                                     font=ImageFont.truetype(str(font_path), size), fill=0)
            meta = {"font_index": font_index, "font_size": size, "text": text}
            evaluate(raw, "negative", "ordinary_text", **meta)
            evaluate(jpeg(raw, 25), "negative", "jpeg_ordinary_text", **meta)
            for block in (3, 5, 8, 11):
                px, py = (block - 1), (block // 2)
                original = mosaic(raw, block, px, py)
                # Crop after pixelation: border cells include unseen outside pixels.
                crop_x, crop_y = 7, 3
                patch = original.crop((crop_x, crop_y, 749, 105))
                expected = {"block": block, "phase_x": (px + crop_x) % block,
                            "phase_y": (py + crop_y) % block}
                noise = rng.integers(-3, 4, size=(patch.height, patch.width))
                noisy = Image.fromarray(np.clip(np.asarray(patch, dtype=np.int16) + noise,
                                                0, 255).astype("uint8"))
                for label, altered in (
                    ("clean", patch), ("noise_3", noisy),
                    ("jpeg_65", jpeg(patch, 65)),
                    ("blur_0.4", patch.filter(ImageFilter.GaussianBlur(0.4))),
                    ("blur_1.0", patch.filter(ImageFilter.GaussianBlur(1.0))),
                ):
                    evaluate(altered, "integer_mosaic", label, expected, **meta)
                scaled = patch.resize((round(patch.width * 1.13), round(patch.height * 1.13)),
                                      Image.Resampling.BICUBIC)
                evaluate(scaled, "unsupported_noninteger_grid", "rescale_1.13",
                         source_grid=expected, **meta)

    for label, image in (
        ("blank", Image.new("L", (600, 100), 255)),
        ("solid_black", Image.new("L", (600, 100), 0)),
        ("noise", Image.fromarray(rng.integers(0, 256, (100, 600), dtype="uint8"))),
    ):
        evaluate(image, "negative", label)
    mask = Image.new("L", (600, 100), 255)
    ImageDraw.Draw(mask).rectangle((15, 20, 585, 80), fill=0)
    evaluate(mask, "negative", "opaque_rectangle")
    evaluate(jpeg(mask, 25), "negative", "jpeg_opaque_rectangle")
    # Nearest-neighbor enlargement really has grid structure, but is not redaction.
    # Report separately: a detected grid alone must never establish concealment.
    enlarged = raw.resize((raw.width // 4, raw.height // 4), Image.Resampling.BOX)
    enlarged = enlarged.resize((enlarged.width * 4, enlarged.height * 4),
                               Image.Resampling.NEAREST)
    evaluate(enlarged, "structured_nonredaction", "nearest_neighbor_enlargement")

    aggregate = defaultdict(lambda: {"count": 0, "detected": 0,
                                     "exact_first": 0, "exact_retained": 0})
    for row in rows:
        counts = aggregate[f'{row["family"]}/{row["label"]}']
        counts["count"] += 1
        for key in ("detected", "exact_first", "exact_retained"):
            counts[key] += int(row[key])
    result = {"schema_version": 1, "binary_sha256": digest(binary),
              "controller_sha256": digest(Path(__file__)),
              "pillow_version": pillow_version, "numpy_version": np.__version__,
              "fonts": [{"path": str(p), "sha256": digest(p)} for p in fonts],
              "seed": 20261019, "summary": dict(aggregate), "cases": rows,
              "interpretation": "Independent grid-detection evaluation, not text recovery. "
              "No detection is a model limitation, not evidence against a candidate. "
              "Integer-grid scoring is undefined after noninteger rescaling. "
              "Real block structure can also arise from ordinary image enlargement."}
    (out / "evaluation.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["summary"], indent=2))


if __name__ == "__main__":
    main()
