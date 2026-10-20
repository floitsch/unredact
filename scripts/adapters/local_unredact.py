# Copyright (C) 2026 Toit contributors.
"""Comparison protocol v1 adapter for this repository's Rust implementation.

Starts from images, performs fresh OCR/calibration, and evaluates supplied targets.
This is neither an external-project adapter nor a reproduction of their behavior.
"""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def intersects(a, b):
    return (a["x"] < b["x"] + b["w"] and b["x"] < a["x"] + a["w"]
            and a["y"] < b["y"] + b["h"] and b["y"] < a["y"] + a["h"])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--single-observation", action="store_true")
    parser.add_argument("--uniform-padding", action="store_true", help="Explicit uniform prior for opaque-mask padding; opt-in scoring model")
    parser.add_argument("--discover-fonts", action="store_true")
    parser.add_argument("--learn-spaces", action="store_true")
    parser.add_argument("--scan-anchors", action="store_true")
    parser.add_argument("--line-spaces", action="store_true")
    parser.add_argument("--mask-aware-ocr", action="store_true")
    parser.add_argument("--ocr-tessdata", type=Path, help="Optional English LSTM model for mask-aware OCR")
    parser.add_argument("--hinted-ppem", help="Comma-separated optional source raster grids")
    parser.add_argument("--neural-config", type=Path)
    parser.add_argument("--font-catalog", type=Path, help="Full-catalog control; preserves default fonts")
    parser.add_argument("--context-model", type=Path, help="Optional local model for a separate contextual ranking")
    parser.add_argument("--context-provider", choices=["bert", "t5"], default="bert")
    parser.add_argument("--context-python", type=Path, help="Python environment with Torch/Transformers")
    parser.add_argument("--paragraph-context", action="store_true")
    parser.add_argument("--word-normalization", action="store_true")
    parser.add_argument("--sequence-score", action="store_true")
    args = parser.parse_args()
    if args.uniform_padding and args.scan_anchors:
        parser.error("--uniform-padding and --scan-anchors select different evidence models")
    if args.line_spaces and not args.scan_anchors:
        parser.error('--line-spaces requires --scan-anchors')
    if args.ocr_tessdata and not args.mask_aware_ocr:
        parser.error('--ocr-tessdata requires --mask-aware-ocr')
    if bool(args.context_model) != bool(args.context_python):
        parser.error('--context-model and --context-python must be supplied together')
    if args.paragraph_context and not args.context_model:
        parser.error('--paragraph-context requires --context-model')
    if args.word_normalization and not args.context_model:
        parser.error('--word-normalization requires --context-model')
    if args.sequence_score and (not args.context_model or args.word_normalization):
        parser.error('--sequence-score requires --context-model and conflicts with --word-normalization')
    request = json.load(sys.stdin)
    if request["schema_version"] != 1 or request["input"]["schema_version"] != 1:
        raise ValueError("Unsupported comparison protocol version")
    data = request["input"]
    work = Path(request["work_dir"])
    work.mkdir(parents=True, exist_ok=False)
    project_dir = work / "project"
    binary = args.binary.resolve()

    def run(*argv):
        proc = subprocess.run([str(binary), *map(str, argv)], capture_output=True, text=True)
        if proc.returncode:
            raise RuntimeError(proc.stderr)
        return proc.stdout

    font_args = []
    if args.font_catalog:
        fonts = set(run("fonts").splitlines())
        for item in json.loads(args.font_catalog.read_text()):
            path = (args.font_catalog.parent / item["path"]).resolve()
            if hashlib.sha256(path.read_bytes()).hexdigest() != item["sha256"]:
                raise ValueError("Font catalog hash mismatch")
            fonts.add(str(path))
        font_args = [arg for path in sorted(fonts) for arg in ("--font", path)]

    targets = data["targets"][:1] if args.single_observation else data["targets"]
    if any(t["evidence"]["kind"] == "native_span" for t in targets):
        print(json.dumps({"schema_version": 1, "status": "unsupported",
                          "reason": "Raster-input track cannot preserve native PDF coordinates",
                          "ranked_groups": [], "ranking_complete": False, "compatible": None}))
        return
    needed_pages = {t["page"] for t in targets}
    mapped = {}
    for index, page in enumerate(data["pages"]):
        if page["id"] not in needed_pages:
            continue
        run("import", "--project", project_dir, "--dpi", page["dpi"], page["image"])
        path = project_dir / "project.json"
        project = json.loads(path.read_text())
        imported = next(p for p in project["pages"] if p["source"] == str(Path(page["image"]).resolve()))
        if args.mask_aware_ocr:
            masks = work / f"masks-{index}.json"
            masks.write_text(json.dumps(page["redaction_regions"]))
            run("refresh-ocr", "--project", project_dir, "--page", imported["id"], "--masks", masks,
                *(["--tessdata", args.ocr_tessdata] if args.ocr_tessdata else []))
            project = json.loads(path.read_text())
            imported = next(p for p in project["pages"] if p["source"] == str(Path(page["image"]).resolve()))
        # Shared input supplies redaction locations, but no OCR labels or typography.
        # Exclude OCR guesses anywhere inside those masks from calibration.
        imported["words"] = [w for w in imported["words"]
                             if not any(intersects(w["bounds"], box) for box in page["redaction_regions"])]
        # Spacing spans must not bridge supplied masks even if OCR found no word
        # inside the mask and rectangle detection missed it.
        if args.learn_spaces or args.neural_config or args.scan_anchors:
            imported["proposed_redactions"].extend(page["redaction_regions"])
        path.write_text(json.dumps(project))
        profile_id = f"profile-{index}"
        run("calibrate", "--project", project_dir, "--page", imported["id"], "--id", profile_id,
            *(["--discover-fonts"] if args.discover_fonts else []),
            *(["--learn-spaces"] if args.learn_spaces else []),
            *(["--hinted-ppem", args.hinted_ppem] if args.hinted_ppem else []),
            *(["--neural-config", args.neural_config] if args.neural_config else []), *font_args)
        mapped[page["id"]] = (imported["id"], profile_id)
    for target in targets:
        target = dict(target)
        if args.uniform_padding and target["evidence"]["kind"] == "opaque":
            target["evidence"] = {**target["evidence"], "kind": "opaque_uniform"}
        target["page"], target["profile"] = mapped[target["page"]]
        target["group"] = "comparison"
        spec = work / "target.json"
        spec.write_text(json.dumps(target))
        run("target", "--project", project_dir, "--spec", spec,
            *(["--scan-anchors"] if args.scan_anchors else []),
            *(["--line-spaces"] if args.line_spaces else []))
    vocabulary = work / "candidates.json"
    vocabulary.write_text(json.dumps({"candidates": data["candidates"]}))
    report = json.loads(run("solve", "--project", project_dir, "--group", "comparison",
                            "--candidates", vocabulary, "--limit", len(data["candidates"])))
    (work / "native-result.json").write_text(json.dumps(report, indent=2))
    contextual = None
    if args.context_model:
        contextual = json.loads(run("context-rank", "--project", project_dir, "--group", "comparison",
            "--candidates", vocabulary, "--provider", args.context_python,
            "--provider-arg", Path(__file__).resolve().parents[1] / ("span_context_provider.py" if args.context_provider == "t5" else "masked_context_provider.py"),
            "--provider-arg=--model-dir", "--provider-arg", args.context_model,
            *(["--paragraph-context"] if args.paragraph_context else []),
            *(["--word-normalization"] if args.word_normalization else []),
            *(["--sequence-score"] if args.sequence_score else [])))
        if contextual['physical'] != report:
            raise ValueError('Context ranking changed the physical report')
        (work / "context-result.json").write_text(json.dumps(contextual, indent=2))
    groups = defaultdict(list)
    for candidate in report["candidates"]:
        if contextual and candidate['compatible']:
            continue
        groups[candidate["evidence_rank_min"]].append(candidate["id"])
    ranked = [groups[r] for r in sorted(groups)]
    if contextual:
        # Context orders compatible candidates; incompatible candidates retain
        # their original physical ordering below them. Neither set changes.
        ranked = contextual['ranked_groups'] + ranked
    print(json.dumps({"schema_version": 1, "status": "ok", "reason": None,
                      "ranked_groups": ranked,
                      "ranking_complete": len(report["candidates"]) == len(data["candidates"]),
                      "compatible": [c["id"] for c in report["candidates"] if c["compatible"]]}))


if __name__ == "__main__":
    main()
