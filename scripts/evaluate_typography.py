# Copyright (C) 2026 Toit contributors.
"""Evaluate frozen size grouping on explicitly selected source PDF scan pages.

Uses original single-image scans, fresh OCR, and the Rust matched-audit diagnostic.
Labels are OCR output, not original-redaction truth. Each page calibrates from its
own visible training words; this is not transfer of a fitted font between pages.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-pdf", type=Path, required=True)
    parser.add_argument("--pages", type=int, nargs="+", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/unredact"))
    parser.add_argument("--audit-binary", type=Path, default=Path("target/release/examples/audit_typography"))
    args = parser.parse_args()
    source, binary, audit = (p.resolve() for p in (args.source_pdf, args.binary, args.audit_binary))
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    provenance = {"source_sha256": digest(source), "binary_sha256": digest(binary),
                  "audit_binary_sha256": digest(audit), "script_sha256": digest(Path(__file__)),
                  "pages": args.pages, "source": str(source), "imports": {}}
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    commands = []

    def run(argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, text=True, timeout=180)
        commands.append({"argv": list(map(str, argv)), "returncode": result.returncode, "stderr": result.stderr})
        (out / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        if result.returncode:
            raise RuntimeError(result.stderr)
        return result.stdout

    listing = run(["pdfimages", "-list", source])
    (out / "images.txt").write_text(listing)
    rows = [line.split() for line in listing.splitlines()[2:]]
    observations, summaries = [], []

    def rmse(key, items):
        return math.sqrt(sum((o[key] - o["observed"]) ** 2 for o in items) / len(items)) if items else None

    for n in args.pages:
        row, = [r for r in rows if int(r[0]) == n]
        if row[2] != "image" or row[12] != row[13]:
            raise ValueError(f"Page {n} is not a single isotropically placed image")
        directory = out / f"page-{n}"
        directory.mkdir()
        run(["pdfimages", "-f", n, "-l", n, "-png", source, directory / "source"])
        image, = directory.glob("source-*.png")
        project = directory / "project"
        run([binary, "import", "--project", project, "--dpi", row[12], image])
        provenance["imports"][str(n)] = {"image_sha256": digest(image),
            "project_sha256": digest(project / "project.json"), "dpi": int(row[12]), "encoding": row[8]}
        result = run([audit, project])
        (out / f"page-{n}.json").write_text(result)
        report, = json.loads(result)
        obs = [o for g in report["groups"] if g["status"] == "ok" for o in g["audit"]]
        shared = [o for o in obs if o["block_prediction"] is not None]
        observations.extend(obs)
        summary = {"page": n, "audit_observations": len(obs),
                   "whole_rmse": rmse("whole_prediction", obs), "group_rmse": rmse("group_prediction", obs),
                   "block_comparable_observations": len(shared), "block_rmse_shared": rmse("block_prediction", shared),
                   "group_rmse_shared": rmse("group_prediction", shared),
                   "failed_groups": [g["reason"] for g in report["groups"] if g["status"] != "ok"],
                   "unassigned_lines": len(report["grouping"]["unassigned_lines"])}
        summaries.append(summary)
        print(json.dumps(summary), flush=True)
    shared = [o for o in observations if o["block_prediction"] is not None]
    summary = {"pages": summaries, "audit_observations": len(observations),
        "pooled_whole_rmse": rmse("whole_prediction", observations),
        "pooled_group_rmse": rmse("group_prediction", observations),
        "block_comparable_observations": len(shared),
        "pooled_block_rmse_shared": rmse("block_prediction", shared),
        "pooled_group_rmse_shared": rmse("group_prediction", shared),
        "interpretation": "Descriptive pooled errors on matched observations; repeated words and within-page "
        "observations are correlated. Ungrouped/uncalibratable regions are reported but excluded. "
        "Each page uses its own visible training words. OCR labels uncorrected; not unknown-redaction recovery."}
    if digest(binary) != provenance["binary_sha256"] or digest(audit) != provenance["audit_binary_sha256"]:
        raise RuntimeError("Executable changed during evaluation")
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")


if __name__ == "__main__":
    main()
