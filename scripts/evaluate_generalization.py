# Copyright (C) 2026 Toit contributors.
"""Stress-test OCR calibration with withheld fonts and scan transformations.

Ground-truth names, widths, and rendering parameters stay in the controller.
The app receives only images, target rectangles, padding assumptions, and a fixed
candidate vocabulary. No font identity is supplied to calibration.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
import time

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont
from evaluate_scans import TEXT, FIRST, LAST
from evaluate_pixelation import digest


def transform_box(box, scale, angle, width, height):
    theta = math.radians(angle)
    cx, cy = width * scale / 2, height / 2
    corners = []
    for x in (box["x"], box["x"] + box["w"]):
        for y in (box["y"], box["y"] + box["h"]):
            dx, dy = x * scale - cx, y - cy
            corners.append((cx + dx * math.cos(theta) + dy * math.sin(theta),
                            cy - dx * math.sin(theta) + dy * math.cos(theta)))
    xs, ys = zip(*corners)
    x, y = math.floor(min(xs)), math.floor(min(ys))
    return {"x": x, "y": y, "w": math.ceil(max(xs)) - x,
            "h": math.ceil(max(ys)) - y}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/unredact"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--discover-fonts", action="store_true")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Use a fresh output directory")
    out = args.output.resolve()
    out.mkdir(parents=True)
    binary = args.binary.resolve()

    def run(*argv):
        p = subprocess.run([str(binary), *map(str, argv)], capture_output=True, text=True)
        if p.returncode:
            raise RuntimeError(p.stderr)
        return p.stdout

    defaults = run("fonts").splitlines()
    source_fonts = [defaults[0], "/usr/share/fonts/fanwood-text/FanwoodText-Regular.ttf",
                    "/usr/share/fonts/grenze/Grenze-Regular.ttf",
                    "/usr/share/fonts/goudy-bookletter-1911/GoudyBookletter1911.ttf"]
    for path in source_fonts:
        if not Path(path).is_file():
            raise RuntimeError(f"Fixture font is missing: {path}")
    conditions = [("control", source_fonts[0], 1.0, 0.0, False),
                  ("unseen_fanwood", source_fonts[1], 1.0, 0.0, False),
                  ("unseen_grenze", source_fonts[2], 1.0, 0.0, False),
                  ("unseen_goudy", source_fonts[3], 1.0, 0.0, False),
                  ("horizontal_stretch", source_fonts[0], 1.15, 0.0, False),
                  ("rotation_2deg", source_fonts[0], 1.0, 2.0, False),
                  ("mixed_fonts", source_fonts[0], 1.0, 0.0, True)]
    vocabulary = [f"{f} {s}" for f in FIRST for s in LAST]
    truths = [vocabulary[(i * 37 + 19) % len(vocabulary)] for i in range(12)]
    candidate_path = out / "candidates.txt"
    candidate_path.write_text("\n".join(vocabulary) + "\n")
    rows = []
    for condition, font_path, scale, angle, mixed in conditions:
        print(f"Evaluating {condition}", flush=True)
        directory = out / condition
        directory.mkdir()
        project_dir = directory / "project"
        font = ImageFont.truetype(font_path, 32)
        alternate = ImageFont.truetype(defaults[2], 32)
        image = Image.new("L", (1900, 2600), 255)
        draw = ImageDraw.Draw(image)
        x, y = 70, 30
        for index, word in enumerate(TEXT.split()):
            active_font = alternate if mixed and index % 2 else font
            width = draw.textlength(word + " ", font=active_font)
            if x + width > 1830:
                x, y = 70, y + 55
            draw.text((x, y), word, font=active_font, fill=0)
            x += width
        assert y < 1400
        boxes = []
        for i, truth in enumerate(truths):
            patch = Image.new("L", (600, 70), 255)
            ImageDraw.Draw(patch).text((10, 0), truth, font=font, fill=0)
            yy, xx = np.where(np.asarray(patch) < 160)
            ink_width = int(xx.max() - xx.min() + 1)
            box = {"x": 100, "y": 1500 + i * 65, "w": ink_width + 6, "h": 43}
            draw.rectangle((box["x"], box["y"], box["x"] + box["w"] - 1,
                            box["y"] + box["h"] - 1), fill=0)
            boxes.append(transform_box(box, scale, angle, image.width, image.height))
        image = image.resize((round(image.width * scale), image.height), Image.Resampling.BICUBIC)
        if angle:
            image = image.rotate(angle, Image.Resampling.BICUBIC, fillcolor=255)
        image = image.filter(ImageFilter.GaussianBlur(0.45))
        path = directory / "page.pgm"
        image.save(path)
        run("import", "--project", project_dir, path)
        project = json.loads((project_dir / "project.json").read_text())
        page = project["pages"][0]
        try:
            started = time.monotonic()
            profile = json.loads(run("calibrate", "--project", project_dir,
                                     "--page", page["id"], "--id", "body",
                                     *(["--discover-fonts"] if args.discover_fonts else [])))
            calibration_seconds = time.monotonic() - started
        except RuntimeError as error:
            rows.append({"condition": condition, "error": str(error), "cases": []})
            continue
        cases = []
        for i, box in enumerate(boxes):
            spec = {"id": f"t-{i}", "page": page["id"], "profile": "body", "bounds": box,
                    "group": f"g-{i}", "surface_kind": "canonical", "evidence": {
                        "kind": "opaque", "padding_min": 4.0 * scale,
                        "padding_max": 8.0 * scale, "measurement_sigma": 1.0},
                    "left_context": "", "right_context": "", "dependence_key": None}
            spec_path = directory / "target.json"
            spec_path.write_text(json.dumps(spec))
            run("target", "--project", project_dir, "--spec", spec_path)
            report = json.loads(run("solve", "--project", project_dir, "--group", f"g-{i}",
                                    "--candidates", candidate_path, "--limit", 400))
            found = next((c for c in report["candidates"] if c["name"] == truths[i]), None)
            cases.append({"truth": truths[i], "compatible": bool(found and found["compatible"]),
                          "unique_first": bool(found and found["evidence_rank_max"] == 1),
                          "rank": [found["evidence_rank_min"], found["evidence_rank_max"]] if found else None,
                          "compatible_candidates": report["compatible_candidates"],
                          "best_name": report["candidates"][0]["name"] if report["candidates"] else None})
        rows.append({"condition": condition, "rendered_font": font_path,
                     "rendered_font_in_default_set": font_path in defaults,
                     "installed_font_search": args.discover_fonts,
                     "calibration_seconds": calibration_seconds,
                     "ocr_words": len(page["words"]), "source_sha256": digest(path),
                     "project_sha256": digest(project_dir / "project.json"),
                     "profile": profile, "truth_compatible": sum(c["compatible"] for c in cases),
                     "unique_first": sum(c["unique_first"] for c in cases),
                     "mean_survivors": sum(c["compatible_candidates"] for c in cases) / len(cases),
                     "cases": cases})
    report = {"schema_version": 1, "binary_sha256": digest(binary),
              "controller_sha256": digest(Path(__file__)), "rows": rows,
              "limitations": ["Synthetic pages, 12 names per condition, one-page evidence.",
                              "Without installed-font search, missing-font and mixed-font conditions deliberately violate the font model.",
                              "Rotated targets use axis-aligned enclosing boxes with unchanged padding assumptions.",
                              "These tests measure compatibility and ambiguity, not identification probabilities."]}
    (out / "evaluation.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps([{k: v for k, v in r.items() if k not in ("profile", "cases")}
                      for r in rows], indent=2))


if __name__ == "__main__":
    main()
