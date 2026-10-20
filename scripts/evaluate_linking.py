# Copyright (C) 2026 Toit contributors.
"""Independent raster/OCR evaluation of passage links and conditional joint solving.

Labels are withheld from the app. Targets contain locations but no supplied
context or cross-document group. Same-template/different-name controls deliberately
cannot be resolved from visible context and must remain ambiguous.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess

from PIL import Image, ImageDraw, ImageFilter, ImageFont
from evaluate_scans import TEXT, FIRST, LAST
from evaluate_pixelation import digest

PAIRS = [
    ("During the quarterly planning meeting", "submitted the revised schedule yesterday"),
    ("Following the annual engineering review", "prepared an updated technical assessment"),
    ("Before the regional transport conference", "distributed the preliminary travel arrangements"),
    ("After the confidential budget discussion", "approved several additional research expenses"),
    ("Within the published committee minutes", "described the proposed laboratory improvements"),
    ("According to the signed attendance sheet", "attended the scheduled afternoon workshop"),
    ("According to the signed attendance sheet", "attended the scheduled afternoon workshop"),
    ("During the quarterly planning meeting", "discussed entirely unrelated historical correspondence"),
    ("By", "today"),
]


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

    fonts = run("fonts").splitlines()[:2]
    assert len(fonts) == 2
    vocabulary = [f"{f} {s}" for f in FIRST for s in LAST]
    candidates = out / "candidates.txt"
    candidates.write_text("\n".join(vocabulary) + "\n")
    truths = [vocabulary[(i * 37 + 19) % len(vocabulary)] for i in range(9)]
    label_rows = []
    for doc, font_path in enumerate(fonts):
        font = ImageFont.truetype(font_path, 30 + doc * 3)
        image = Image.new("L", (2100, 2300), 255)
        draw = ImageDraw.Draw(image)
        x, y = 40, 30
        for word in TEXT.split():
            width = draw.textlength(word + " ", font=font)
            if x + width > 2040:
                x, y = 40, y + 55
            draw.text((x, y), word, font=font, fill=0)
            x += width
        assert y < 1350
        targets = []
        order = list(range(9)) if doc == 0 else [3, 0, 6, 2, 8, 4, 5, 1, 7]
        for row, case in enumerate(order):
            left, right = PAIRS[case]
            if doc == 1 and case == 7:
                right = "filed another completely different administrative request"
            truth = truths[case]
            # Deliberately swap template values across documents: context alone
            # cannot distinguish the two concealed identities.
            if doc == 1 and case in (5, 6):
                truth = truths[11 - case]
            baseline = 1500 + row * 75
            draw.text((40, baseline), left, font=font, fill=0, anchor="ls")
            start = 40 + draw.textlength(left + " ", font=font)
            ink = font.getbbox(truth, anchor="ls")
            width = ink[2] - ink[0] + 6
            x = round(start)
            box = {"x": x, "y": baseline - 36, "w": width, "h": 44}
            draw.rectangle((x, baseline - 36, x + width - 1, baseline + 7), fill=0)
            draw.text((x + width + 12, baseline), right, font=font, fill=0, anchor="ls")
            targets.append((case, truth, box))
        image = image.filter(ImageFilter.GaussianBlur(0.35))
        path = out / f"document-{doc}.pgm"
        image.save(path)
        run("import", "--project", project_dir, path)
        project = json.loads((project_dir / "project.json").read_text())
        page = next(p for p in project["pages"] if p["source"] == str(path))
        excluded = {w["block"] for w in page["words"] if w["bounds"]["y"] > 1400}
        counts = Counter(w["block"] for w in page["words"] if w["block"] not in excluded)
        block = counts.most_common(1)[0][0]
        profile = f"profile-{doc}"
        run("calibrate", "--project", project_dir, "--page", page["id"],
            "--block", block, "--id", profile)
        for case, truth, box in targets:
            target_id = f"doc-{doc}-target-{case}"
            spec = {"id": target_id, "page": page["id"], "bounds": box,
                    "profile": profile, "group": target_id, "surface_kind": "canonical",
                    "evidence": {"kind": "opaque", "padding_min": 4, "padding_max": 8,
                                 "measurement_sigma": 1},
                    "left_context": "", "right_context": "", "dependence_key": None}
            spec_path = out / "target.json"
            spec_path.write_text(json.dumps(spec))
            run("target", "--project", project_dir, "--spec", spec_path)
            label_rows.append({"id": target_id, "case": case, "truth": truth,
                               "source_sha256": digest(path)})
    links = json.loads(run("link-targets", "--project", project_dir))
    (out / "links.json").write_text(json.dumps(links, indent=2) + "\n")
    predicted = {frozenset((l["a"], l["b"])) for l in links["links"]}
    unique_expected = {frozenset((f"doc-0-target-{i}", f"doc-1-target-{i}")) for i in range(5)}
    template_expected = {frozenset((f"doc-0-target-{i}", f"doc-1-target-{j}"))
                         for i in (5, 6) for j in (5, 6)}
    expected = unique_expected | template_expected
    template_links = [l for l in links["links"] if frozenset((l["a"], l["b"])) in template_expected]
    scenario_summary = []
    for case in (0, 5):
        anchor = f"doc-0-target-{case}"
        solved = json.loads(run("solve-links", "--project", project_dir, "--anchor", anchor,
                                "--candidates", candidates, "--limit", 400))
        (out / f"scenarios-{case}.json").write_text(json.dumps(solved, indent=2) + "\n")
        for scenario in solved["scenarios"]:
            result = scenario["result"]
            truth = next((c for c in result["candidates"] if c["name"] == truths[case]), None) if result else None
            scenario_summary.append({"anchor": anchor, "targets": scenario["targets"],
                                     "dependence": scenario["dependence_assumption"],
                                     "compatible_candidates": result["compatible_candidates"] if result else None,
                                     "truth_compatible": truth["compatible"] if truth else False,
                                     "truth_evidence_rank": [truth["evidence_rank_min"], truth["evidence_rank_max"]] if truth else None,
                                     "error": scenario["error"]})
    result = {"schema_version": 1, "binary_sha256": digest(binary),
              "controller_sha256": digest(Path(__file__)), "project_sha256": digest(project_dir / "project.json"),
              "targets": len(label_rows), "expected_unique_pairs": 5,
              "unique_pairs_found": len(predicted & unique_expected),
              "expected_template_pairs": 4, "template_pairs_found": len(predicted & template_expected),
              "template_pairs_marked_ambiguous": sum(bool(l["competing_a"] and l["competing_b"]) for l in template_links),
              "unexpected_pairs": [sorted(p) for p in predicted - expected],
              "insufficient_context": links["insufficient_context"], "scenarios": scenario_summary,
              "labels": label_rows,
              "limitations": ["Synthetic raster documents, two installed fonts, supplied target positions.",
                              "No transitive/multi-hop inference; contexts share wording and single-line layout.",
                              "Template matches deliberately do not identify the concealed value.",
                              "No real-document or external-project comparison."]}
    (out / "evaluation.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k != "labels"}, indent=2))


if __name__ == "__main__":
    main()
