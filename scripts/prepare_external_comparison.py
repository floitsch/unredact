# Copyright (C) 2026 Toit contributors.
"""Predeclare independent contextual scan fixtures and pinned external adapters.

Single-occurrence primary track. Known mask padding is removed identically for
all tools; residual width sigma is fixed at 1.5px. No font/OCR labels enter adapter
requests. Truth-absent controls use the identical image. No results are read here.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

from evaluate_scans import FIRST, LAST, TEXT


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cases-per-font", type=int, default=8)
    parser.add_argument("--family", action="append", help="Fontconfig pattern; repeat for predeclared styles/families")
    parser.add_argument("--geometry", choices=["ink", "advance"], default="ink",
                        help="Declared width convention; advance keeps right context at its natural origin")
    parser.add_argument("--seed", type=int, default=7361)
    args = parser.parse_args()
    if not 1 <= args.cases_per_font <= 8:
        parser.error("cases-per-font must be 1..8")
    root = Path(__file__).resolve().parents[1]
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    (out / "images").mkdir()
    candidates = [{"id": hashlib.sha256(f"{f} {s}".encode()).hexdigest()[:16],
                   "canonical_name": f"{f} {s}", "surfaces": [{"text": f"{f} {s}", "kind": "canonical"}],
                   "source_ids": [], "context_score": None} for f in FIRST for s in LAST]
    rng = np.random.default_rng(args.seed)
    chosen = rng.choice(len(candidates), size=8, replace=False)[:args.cases_per_font]
    families = args.family or ["Liberation Serif", "Liberation Sans", "DejaVu Serif"]
    cases = []
    ground_truth = []
    for fi, family in enumerate(families):
        font_path = subprocess.check_output(["fc-match", "-f", "%{file}", family], text=True)
        size = 32 + 3 * fi
        font = ImageFont.truetype(font_path, size=size)
        image = Image.new("L", (1800, 2400), 255)
        draw = ImageDraw.Draw(image)
        x, y = 40, 30
        for word in TEXT.split():
            advance = draw.textlength(word + " ", font=font)
            if x + advance > 1740:
                x, y = 40, y + 55
            draw.text((x, y), word, font=font, fill=0)
            x += advance
        target_start = max(1400, int(np.ceil((y + 100) / 100)) * 100)
        required_height = target_start + 100 * args.cases_per_font
        if required_height > image.height:
            expanded = Image.new("L", (image.width, required_height), 255)
            expanded.paste(image, (0, 0))
            image = expanded
            draw = ImageDraw.Draw(image)
        targets = []
        for ci, chosen_index in enumerate(chosen):
            truth = candidates[int(chosen_index)]
            left = "The correspondence identifies "
            right = " as the designated contact."
            y = target_start + 100 * ci
            draw.text((40, y), left, font=font, fill=0)
            x = int(np.ceil(40 + draw.textlength(left, font=font)))
            # Rasterized ink extent is measured independently of every solver.
            patch = Image.new("L", (900, 100), 255)
            ImageDraw.Draw(patch).text((10, 0), truth["canonical_name"], font=font, fill=0)
            yy, xx = np.where(np.asarray(patch) < 160)
            ink_width = int(xx.max() - xx.min() + 1)
            width = float(ink_width)
            if args.geometry == "advance":
                width = float(draw.textlength(truth["canonical_name"], font=font))
                # Keep the natural text origin; the mask covers its advance with
                # three pixels of padding on each side, without moving context.
                box = {"x": float(x - 3), "y": float(y),
                       "w": float(np.ceil(width) + 6), "h": 60.0}
                draw.rectangle((x - 3, y, x + np.ceil(width) + 2, y + 59), fill=0)
                draw.text((x + width, y), right, font=font, fill=0)
            else:
                box = {"x": float(x), "y": float(y), "w": float(ink_width + 6), "h": 60.0}
                draw.rectangle((x, y, x + ink_width + 5, y + 59), fill=0)
                draw.text((x + ink_width + 6, y), right, font=font, fill=0)
            targets.append({"id": f"target-{fi}-{ci}", "page": f"page-{fi}", "bounds": box,
                            "surface_kind": "canonical", "evidence": {"kind": "width", "width": width,
                            "measurement_sigma": 1.5}, "left_context": "", "right_context": "",
                            "dependence_key": None})
        image = image.filter(ImageFilter.GaussianBlur(0.45))
        noise = np.random.default_rng(args.seed + 1000 + fi).normal(0, 1.8, (image.height, image.width))
        image = Image.fromarray(np.clip(np.asarray(image, dtype=float) + noise, 0, 255).astype("uint8"))
        path = out / "images" / f"page-{fi}.pgm"
        image.save(path)
        page = {"id": f"page-{fi}", "image": str(path.relative_to(out)), "image_sha256": digest(path),
                "dpi": 200.0, "redaction_regions": [t["bounds"] for t in targets]}
        for ci, target in enumerate(targets):
            truth = candidates[int(chosen[ci])]
            data = {"schema_version": 1, "pages": [page], "targets": [target], "candidates": candidates}
            cases.append({"id": f"present-{fi}-{ci}", "truth_id": truth["id"], "input": data})
            absent = {**data, "candidates": [c for c in candidates if c["id"] != truth["id"]]}
            cases.append({"id": f"absent-{fi}-{ci}", "truth_id": truth["id"], "input": absent})
        ground_truth.append({"page": page["id"], "font": font_path, "font_sha256": digest(Path(font_path)),
                             "size": size, "padding": 6,
                             "names": [candidates[int(i)]["canonical_name"] for i in chosen]})
    suite = {"schema_version": 1, "track": "candidate_ranking_from_images",
             "description": f"Predeclared contextual scan fixtures; 400 synthetic names; {len(families)} fonts; "
             f"single occurrence; common padding-corrected {args.geometry}-width measurement and sigma 1.5px; "
             "independent Pillow rendering, blur .45px and noise 1.8; truth-absent controls.", "cases": cases}
    (out / "suite.json").write_text(json.dumps(suite, indent=2) + "\n")
    python = root / "runs/baseline-venv/bin/python"
    scripts = root / "scripts/adapters"
    tools = root / "runs/baseline-tools"
    shared = [scripts / "external_common.py", out / "python-environment.txt"]
    environment = subprocess.check_output([str(python), "-c",
        "import importlib.metadata as m; print('\\n'.join(sorted(f'{d.metadata[\"Name\"]}=={d.version}' for d in m.distributions())))"], text=True)
    (out / "python-environment.txt").write_text(environment)
    adapters = [{"id": "local-single", "kind": "local", "executable": str(python),
                 "args": [str(scripts / "local_unredact.py"), "--binary", str(root / "target/release/unredact")],
                 "artifacts": [str(scripts / "local_unredact.py"), str(root / "target/release/unredact")],
                 "repository": None}]
    configurations = [
        ("alex-gilbert-unredact", "baseline-alex", "alex_browser.py", [
            "--wasm-package", str(tools / "alex-pkg"), "--browser-deps", str(tools / "browser-deps/node_modules"),
            "--browser", "/opt/google/chrome/chrome"],
            [tools / "alex-pkg/unredact_core.js", tools / "alex-pkg/unredact_core_bg.wasm",
             tools / "browser-deps/package-lock.json", Path("/opt/google/chrome/chrome")]),
        ("mortenfj-unredactron", "baseline-morten", "morten_unredactron.py", [], []),
    ]
    for identity, repository, script, extra, artifacts in configurations:
        repo = root / repository
        commit = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
        adapters.append({"id": identity, "kind": "external", "executable": str(python),
                         "args": [str(scripts / script), "--repository", str(repo), *extra],
                         "artifacts": list(map(str, [scripts / script, *shared, *artifacts])),
                         "repository": {"directory": str(repo), "commit": commit}})
    # Fixed sensitivity setting: same solver, widest normal UI tolerance (5 + .1).
    wide = json.loads(json.dumps(adapters[1]))
    wide["id"] = "alex-gilbert-unredact-wide"
    wide["args"] += ["--tolerance", "5.1"]
    adapters.append(wide)
    manifest = {"schema_version": 1, "required_external": [c[0] for c in configurations], "adapters": adapters}
    (out / "adapters.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (out / "generator-provenance.json").write_text(json.dumps({
        "generator_sha256": digest(Path(__file__)), "suite_sha256": digest(out / "suite.json"),
        "ground_truth": ground_truth, "candidate_seed": args.seed, "geometry": args.geometry,
        "limitations": "Controlled single-font pages, known padding and mask locations. "
        "Not an Epstein-document accuracy estimate, language-model evaluation, or joint comparison."
    }, indent=2) + "\n")
    print(out / "suite.json")


if __name__ == "__main__":
    main()
