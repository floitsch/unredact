# Copyright (C) 2026 Toit contributors.
"""Export an independent evaluate_scans run into the matched-image comparison track.

Copies only raster pages and supplies target/mask locations plus candidate names.
Original fonts, OCR labels, learned profiles, and truth labels are not in requests.
Labels stay in the evaluator's suite file. Adds true-name-absent controls.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scan-evaluation", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=12)
    parser.add_argument("--binary", type=Path, default=Path("target/release/unredact"))
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Use a fresh output directory")
    source = args.scan_evaluation.resolve()
    evaluation = json.loads((source / "evaluation.json").read_text())
    project = json.loads((source / "project/project.json").read_text())
    if not 1 <= args.cases <= len(evaluation["results"]):
        parser.error("Invalid case count")
    out = args.output.resolve()
    (out / "images").mkdir(parents=True)
    pages = []
    for page in project["pages"]:
        path = source / "project" / page["image"]
        destination = out / "images" / f'{page["id"]}.pgm'
        shutil.copyfile(path, destination)
        pages.append({"id": page["id"], "image": str(destination.relative_to(out)),
                      "image_sha256": digest(destination), "dpi": page["dpi"],
                      "redaction_regions": [t["bounds"] for t in project["targets"] if t["page"] == page["id"]]})
    vocabulary = (source / "candidates.txt").read_text().splitlines()
    candidates = [{"id": hashlib.sha256(name.encode()).hexdigest()[:16], "canonical_name": name,
                   "surfaces": [{"text": name, "kind": "canonical"}], "source_ids": [], "context_score": None}
                  for name in vocabulary if name.strip()]
    cases = []
    for index, result in enumerate(evaluation["results"][:args.cases]):
        truth_id = next(c["id"] for c in candidates if c["canonical_name"] == result["truth"])
        targets = [{k: v for k, v in t.items() if k not in ("profile", "group")}
                   for t in project["targets"] if t["group"] == f"case-{index}"]
        if not targets:
            raise ValueError(f"No targets for case {index}")
        input_data = {"schema_version": 1, "pages": pages, "targets": targets, "candidates": candidates}
        cases.append({"id": f"present-{index}", "input": input_data, "truth_id": truth_id})
        absent = {**input_data, "candidates": [c for c in candidates if c["id"] != truth_id]}
        cases.append({"id": f"absent-{index}", "input": absent, "truth_id": truth_id})
    suite = {"schema_version": 1, "track": "candidate_ranking_from_images",
             "description": "Independent Pillow/raster-PDF/OCR fixtures exported as identical raster inputs. "
             "Target/mask locations, padding assumptions, cross-document links, and vocabulary supplied. "
             "No typography/OCR/truth labels in adapter requests. Name-absent controls included.", "cases": cases}
    (out / "suite.json").write_text(json.dumps(suite, indent=2) + "\n")
    adapter = Path("scripts/adapters/local_unredact.py").resolve()
    binary = args.binary.resolve()
    artifacts = [str(adapter), str(binary)]
    tools = []
    for name, kind, extra in (("local-joint", "local", []),
                               ("local-single", "ablation", ["--single-observation"])):
        tools.append({"id": name, "kind": kind, "executable": sys.executable,
                      "args": [str(adapter), "--binary", str(binary), *extra],
                      "artifacts": artifacts, "repository": None})
    manifest = {"schema_version": 1,
                "required_external": ["alex-gilbert-unredact", "mortenfj-unredactron"], "adapters": tools}
    (out / "adapters.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (out / "export-provenance.json").write_text(json.dumps({
        "source_evaluation_sha256": digest(source / "evaluation.json"),
        "source_project_sha256": digest(source / "project/project.json"),
        "exporter_sha256": digest(Path(__file__)), "suite_sha256": digest(out / "suite.json"),
    }, indent=2) + "\n")
    print(out / "suite.json")


if __name__ == "__main__":
    main()
