# Copyright (C) 2026 Toit contributors.
"""Audit optional spacing calibration on original Senate report scan images.

This measures visible OCR-labelled pair widths, not unknown-redaction recovery.
Pages are predeclared, with three binary scans and three JPEG scans in the
archived 2014 report. No font, OCR label, or target width is supplied to calibration.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-pdf", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/unredact"))
    args = parser.parse_args()
    binary = args.binary.resolve()
    source = args.source_pdf.resolve()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    pages = [34, 35, 36, 100, 136, 194]
    log = []

    def run(argv):
        result = subprocess.run(list(map(str, argv)), capture_output=True, text=True, timeout=180)
        log.append({"argv": list(map(str, argv)), "returncode": result.returncode,
                    "stderr": result.stderr})
        (out / "commands.json").write_text(json.dumps(log, indent=2) + "\n")
        if result.returncode:
            raise RuntimeError(result.stderr)
        return result.stdout

    listing = run(["pdfimages", "-list", source])
    (out / "images.txt").write_text(listing)
    images = [line.split() for line in listing.splitlines()[2:]]
    records = []
    provenance = {"source": str(source), "source_sha256": digest(source),
                  "binary_sha256": digest(binary), "script_sha256": digest(Path(__file__)),
                  "pages": pages, "selection": "Predeclared PDF page numbers; all page and >=60-word OCR block profiles",
                  "limitations": "Visible OCR labels are not manually corrected. Per-page word-identity splits, "
                  "not independent whole-page generalization. Overlapping pairs are correlated. "
                  "Whole-page and block profiles overlap; do not pool them as independent evidence. "
                  "No unknown redaction content is evaluated."}
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    for number in pages:
        image_rows = [r for r in images if int(r[0]) == number]
        if len(image_rows) != 1 or image_rows[0][12:14] != ["400", "400"]:
            raise ValueError(f"Page {number} is not the expected single 400-DPI image")
        directory = out / f"page-{number}"
        directory.mkdir()
        run(["pdfimages", "-f", number, "-l", number, "-png", source, directory / "source"])
        image, = directory.glob("source-*.png")
        project = directory / "project"
        run([binary, "import", "--project", project, "--dpi", 400, image])
        imported = json.loads((project / "project.json").read_text())["pages"][0]
        blocks = Counter(w["block"] for w in imported["words"] if w["confidence"] >= 85)
        for block in [None, *sorted(b for b, n in blocks.items() if n >= 60)]:
            label = "whole-page" if block is None else "block-" + block.replace(":", "-")
            record = {"page": number, "block": block, "encoding": image_rows[0][8],
                      "image_sha256": digest(image), "profile": str(directory / (label + ".json"))}
            try:
                profile = json.loads(run([binary, "calibrate", "--project", project,
                    "--page", imported["id"], "--id", label, "--learn-spaces",
                    *([] if block is None else ["--block", block])]))
                (directory / (label + ".json")).write_text(json.dumps(profile, indent=2) + "\n")
                record.update({"status": "ok", "reason": profile["spacing"]["reason"],
                               "fits": [{"font": f["font"], "kern": f["kern"], "size": f["size"],
                                         "word_validation_rmse": f["validation_rmse"],
                                         "spacing": f["spacing"]} for f in profile["fits"]]})
            except RuntimeError as error:
                record.update({"status": "error", "reason": str(error)})
            records.append(record)
            (out / "results.json").write_text(json.dumps(records, indent=2) + "\n")
            first = record.get("fits", [{}])[0].get("spacing")
            print(number, label, record["status"], json.dumps(first), flush=True)
    if digest(binary) != provenance["binary_sha256"]:
        raise RuntimeError("Binary changed during evaluation")


if __name__ == "__main__":
    main()
