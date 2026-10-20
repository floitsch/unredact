# Copyright (C) 2026 Toit contributors.
"""Independent mosaic recovery with OCR-learned typography and inferred grids.

Pillow creates calibration and target images. The application sees visible
calibration text, target locations/origins, and a 400-name vocabulary. No font
identity, font size, grid size/phase, or correct-name label is given to it.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

from evaluate_pixelation import digest, jpeg, mosaic
from evaluate_scans import TEXT, FIRST, LAST


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
    project_dir = out / "project"

    def run(*argv):
        return subprocess.check_output([str(binary), *map(str, argv)], text=True)

    fonts = run("fonts").splitlines()[:3]
    vocabulary = [f"{f} {s}" for f in FIRST for s in LAST]
    truths = [vocabulary[i] for i in (19, 56, 93, 130)]
    candidates = out / "candidates.txt"
    candidates.write_text("\n".join(vocabulary) + "\n")
    rng = np.random.default_rng(20261019)
    records = []
    for font_index, font_path in enumerate(fonts):
        font = ImageFont.truetype(font_path, 30 + font_index * 3)
        reference = Image.new("L", (1800, 1450), 255)
        draw = ImageDraw.Draw(reference)
        x, y = 40, 30
        for word in TEXT.split():
            advance = draw.textlength(word + " ", font=font)
            if x + advance > 1740:
                x, y = 40, y + 55
            draw.text((x, y), word, font=font, fill=0)
            x += advance
        assert y + 70 < reference.height
        reference = reference.filter(ImageFilter.GaussianBlur(0.3))
        profile_id = f"profile-{font_index}"
        page_image = Image.new("L", (1800, 2200), 255)
        page_image.paste(reference, (0, 0))
        for case, truth in enumerate(truths):
            raw = Image.new("L", (360, 80), 255)
            ImageDraw.Draw(raw).text((8.25, 48.25), truth, font=font, fill=0, anchor="ls")
            block = (3, 5, 8)[font_index]
            patch = mosaic(raw, block, block - 1, block // 2)
            if case == 1:
                noise = rng.integers(-3, 4, (80, 360))
                patch = Image.fromarray(np.clip(np.asarray(patch, dtype=np.int16) + noise,
                                                0, 255).astype("uint8"))
            elif case == 2:
                patch = jpeg(patch, 65)
            elif case == 3:
                patch = patch.filter(ImageFilter.GaussianBlur(0.4))
            page_image.paste(patch, (20, 1520 + case * 110))
        target_page = out / f"targets-{font_index}.pgm"
        page_image.save(target_page)
        run("import", "--project", project_dir, target_page)
        project = json.loads((project_dir / "project.json").read_text())
        page = next(p for p in project["pages"] if p["source"] == str(target_page))
        # Select an OCR block entirely within the visible reference area. No word
        # transcriptions, font identity, or ground-truth boxes are sent to calibration.
        excluded_blocks = {w["block"] for w in page["words"] if w["bounds"]["y"] >= 1450}
        counts = Counter(w["block"] for w in page["words"] if w["block"] not in excluded_blocks)
        if not counts:
            raise RuntimeError("OCR did not separate visible calibration text from mosaics")
        calibration_block = counts.most_common(1)[0][0]
        run("calibrate", "--project", project_dir, "--page", page["id"],
            "--id", profile_id, "--block", calibration_block)
        for case, truth in enumerate(truths):
            spec = {"id": f"target-{case}-{font_index}", "page": page["id"],
                    "bounds": {"x": 20, "y": 1520 + case * 110, "w": 360, "h": 80},
                    "profile": profile_id, "group": f"case-{case}",
                    "surface_kind": "canonical", "evidence": {
                        "kind": "mosaic_auto", "min_block": 2, "max_block": 12,
                        "origin_x": 8.0, "baseline_y": 48.0, "noise_sigma": 8.0},
                    "left_context": "", "right_context": "", "dependence_key": None}
            spec_path = out / "target.json"
            spec_path.write_text(json.dumps(spec))
            run("target", "--project", project_dir, "--spec", spec_path)
            records.append({"target_id": spec["id"], "truth": truth,
                            "source_sha256": digest(target_page),
                            "true_grid": {"block": block, "phase_x": block - 1,
                                          "phase_y": block // 2}})

    rows = []
    for case, truth in enumerate(truths):
        proc = subprocess.run([str(binary), "solve", "--project", str(project_dir),
                               "--group", f"case-{case}", "--candidates", str(candidates),
                               "--limit", "400"], capture_output=True, text=True)
        if proc.returncode:
            rows.append({"truth": truth, "error": proc.stderr, "unique_top1": False,
                         "truth_compatible": False})
            continue
        report = json.loads(proc.stdout)
        (out / f"result-{case}.json").write_text(proc.stdout)
        candidate = next((c for c in report["candidates"] if c["name"] == truth), None)
        rows.append({"truth": truth, "degradation": ("clean", "noise_3", "jpeg_65", "blur_0.4")[case],
                     "unique_top1": candidate is not None and candidate["evidence_rank_max"] == 1,
                     "evidence_rank": None if candidate is None else
                     [candidate["evidence_rank_min"], candidate["evidence_rank_max"]],
                     "truth_compatible": candidate is not None and candidate["compatible"],
                     "compatible_candidates": report["compatible_candidates"],
                     "truth_pixel_rmse": [] if candidate is None else
                     [o["image_rmse"] for o in candidate["observations"]],
                     "grid_correct": all(report["pixelation_analysis"][r["target_id"]]
                                         ["hypotheses"][0]["grid"] == r["true_grid"]
                                         for r in records if r["truth"] == truth)})
    result = {"schema_version": 1, "binary_sha256": digest(binary),
              "controller_sha256": digest(Path(__file__)), "vocabulary_sha256": digest(candidates),
              "project_sha256": digest(project_dir / "project.json"),
              "cases": rows, "fixtures": records,
              "unique_top1": sum(r["unique_top1"] for r in rows),
              "truth_compatible": sum(r["truth_compatible"] for r in rows),
              "limitations": ["Four synthetic names, 400 candidates, three installed fonts.",
                              "Target locations, approximate origins, and correct links supplied.",
                              "Grid detection can fail on larger blocks or rescaled images.",
                              "No comparison against external projects; no calibrated probabilities."]}
    (out / "evaluation.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k != "fixtures"}, indent=2))


if __name__ == "__main__":
    main()
