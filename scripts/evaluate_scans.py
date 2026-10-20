# Copyright (C) 2026 Toit contributors.
"""Independent Pillow/FreeType fixtures -> scanned PDF -> Poppler -> OCR -> Rust solver.

This evaluates the actual import/calibration path, with no ground-truth word boxes
provided to the app. Ground truth stays in this benchmark controller.
Requires Pillow with RAQM support, numpy, and the app's normal native dependencies.
"""
import argparse
import json
from pathlib import Path
import subprocess

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont


TEXT = """Careful analysis requires reliable measurements and independent validation.
The original document contains ordinary words about scientific research and public
administration. Several different observations help explain the available evidence.
Typography includes proportional character widths and adjustments between adjacent
letters. Printed pages change during scanning because rotation compression blur
and noise affect their appearance. Every estimate should reflect these limitations.
Measurements taken from visible paragraphs provide useful information about the
underlying typeface. Compare possible explanations against separate examples before
accepting a result. A candidate might fit one location while failing another.
Repeated copies provide little additional knowledge unless their sources differ.
The process examines reports schedules correspondence summaries decisions reference
material and historical records. Context helps generate alternatives but cannot
establish the identity of hidden text without supporting physical evidence.
Uncertainty increases when names are absent from dictionaries or redaction padding
is unknown. Reliable tools preserve ambiguous results rather than forcing answers.
Review findings using transparent methods with reproducible experimental controls.
Language models can suggest phrases that satisfy grammatical constraints while
rendering engines check spatial compatibility. Distinct fonts offer complementary
constraints across related documents. Accurate prediction depends on understanding
which observations share systematic errors and which carry independent information.
"""
FIRST = ["Amber", "Avery", "Blair", "Casey", "Drew", "Ellis", "Finley", "Harper",
         "Jamie", "Jordan", "Morgan", "Parker", "Quinn", "Reese", "Riley", "Robin",
         "Rowan", "Sage", "Taylor", "Wren"]
LAST = ["Vale", "Alden", "Briar", "Cedar", "Dale", "Elm", "Frost", "Glen", "Hart",
        "Ives", "Lake", "Marsh", "North", "Oak", "Pine", "Reed", "Stone", "Valehart",
        "West", "Wood"]


def main():
    args = argparse.ArgumentParser()
    args.add_argument("--binary", type=Path, default=Path("target/release/unredact"))
    args.add_argument("--output", type=Path, required=True)
    args.add_argument("--cases", type=int, default=12)
    a = args.parse_args()
    if not 1 <= a.cases <= 16:
        args.error("--cases must be 1–16")
    out = a.output.resolve()
    if out.exists():
        args.error("Use a fresh output directory")
    out.mkdir(parents=True)
    binary = a.binary.resolve()

    def run(*argv):
        proc = subprocess.run([str(binary), *map(str, argv)], capture_output=True, text=True)
        if proc.returncode:
            raise RuntimeError(proc.stderr)
        return proc.stdout

    fonts = run("fonts").splitlines()[:3]
    assert len(fonts) == 3
    vocabulary = [f"{f} {s}" for f in FIRST for s in LAST]
    truths = [vocabulary[(i * 37 + 19) % len(vocabulary)] for i in range(a.cases)]
    candidates_path = out / "candidates.txt"
    candidates_path.write_text("\n".join(vocabulary))
    project_dir = out / "project"
    target_specs = []
    calibration_errors = []
    for i, font_path in enumerate(fonts):
        font = ImageFont.truetype(font_path, size=30 + 3 * i)
        image = Image.new("L", (1800, 2600), 255)
        draw = ImageDraw.Draw(image)
        x, y = 40, 30
        line_height = 55
        for word in TEXT.split():
            advance = draw.textlength(word + " ", font=font)
            if x + advance > 1740:
                x, y = 40, y + line_height
            draw.text((x, y), word, font=font, fill=0)
            x += advance
        assert y < 1450
        boxes = []
        for j, truth in enumerate(truths):
            x, y = 40, 1500 + j * 60
            # Determine actual ink, independent of Rust's width estimator.
            patch = Image.new("L", (700, 70), 255)
            ImageDraw.Draw(patch).text((10, 0), truth, font=font, fill=0)
            pixels = np.asarray(patch)
            yy, xx = np.where(pixels < 160)
            width = int(xx.max() - xx.min() + 1)
            box = {"x": float(x), "y": float(y), "w": float(width + 6), "h": 43.0}
            draw.rectangle((x, y, x + width + 5, y + 42), fill=0)
            boxes.append(box)
        image = image.filter(ImageFilter.GaussianBlur(0.45))
        noise = np.random.default_rng(2026 + i).normal(0, 1.8, (2600, 1800))
        image = Image.fromarray(np.clip(np.asarray(image, dtype=float) + noise, 0, 255).astype("uint8"))
        pdf = out / f"scan-{i}.pdf"
        image.convert("RGB").save(pdf, "PDF", resolution=200.0)
        run("import", "--project", project_dir, "--dpi", "200", pdf)
        project = json.loads((project_dir / "project.json").read_text())
        page = next(p for p in project["pages"] if p["source"] == str(pdf))
        assert page["likely_scan"] and not page["native_chars"]
        profile_id = f"scan-profile-{i}"
        profile = json.loads(run("calibrate", "--project", project_dir, "--page", page["id"], "--id", profile_id))
        calibration_errors.append(profile["fits"][0]["validation_rmse"])
        for j, box in enumerate(boxes):
            spec = {
                "id": f"scan-target-{j}-{i}", "page": page["id"], "bounds": box,
                "profile": profile_id, "group": f"case-{j}", "surface_kind": "canonical",
                "evidence": {"kind": "opaque", "padding_min": 4.0, "padding_max": 8.0,
                             "measurement_sigma": 1.0},
                "left_context": "", "right_context": "", "dependence_key": None,
            }
            path = out / "target.json"
            path.write_text(json.dumps(spec))
            run("target", "--project", project_dir, "--spec", path)
            target_specs.append(spec)

    rows = []
    for j, truth in enumerate(truths):
        report = json.loads(run("solve", "--project", project_dir, "--group", f"case-{j}",
                                "--candidates", candidates_path, "--limit", "400"))
        ranked = report["candidates"]
        index = next((k for k, c in enumerate(ranked) if c["name"] == truth), None)
        rows.append({"truth": truth, "rank": None if index is None else index + 1,
                     "evidence_rank_min": None if index is None else ranked[index]["evidence_rank_min"],
                     "evidence_rank_max": None if index is None else ranked[index]["evidence_rank_max"],
                     "retained": index is not None and ranked[index]["compatible"],
                     "compatible_candidates": report["compatible_candidates"]})
    result = {
        "scope": "Independent Pillow rendering, blur/noise, raster-only PDF, Poppler rendering, Tesseract OCR, automatic font calibration, joint Rust scoring",
        "cases": a.cases, "dictionary_size": len(vocabulary),
        "top1": sum(r["rank"] == 1 for r in rows), "top5": sum(r["rank"] is not None and r["rank"] <= 5 for r in rows),
        "unique_evidence_top1": sum(r["evidence_rank_max"] == 1 for r in rows),
        "truth_retained": sum(r["retained"] for r in rows),
        "mean_survivors": sum(r["compatible_candidates"] for r in rows) / a.cases,
        "validation_rmse_pixels": calibration_errors, "results": rows,
        "limitations": ["Synthetic ground truth, installed font families, correct cross-document links.",
                        "Padding is constrained to 4–8 pixels; wider unknown padding is harder.",
                        "No comparison against executions of the third-party projects."],
    }
    (out / "evaluation.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
