# Copyright (C) 2026 Toit contributors.
"""Join the contextual suite's same-name occurrences into matched joint inputs.

Runs our native joint solver and explicitly labeled external + joint adaptations.
No claim that the upstream projects themselves implement these aggregations.
"""
import argparse
from collections import defaultdict
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source = args.source.resolve()
    suite = json.loads((source / "suite.json").read_text())
    original = json.loads((source / "adapters.json").read_text())
    buckets = defaultdict(list)
    for case in suite["cases"]:
        present = any(c["id"] == case["truth_id"] for c in case["input"]["candidates"])
        buckets[(case["truth_id"], present)].append(case)
    cases = []
    for index, ((truth, present), grouped) in enumerate(sorted(buckets.items())):
        vocabulary = grouped[0]["input"]["candidates"]
        if any(case["input"]["candidates"] != vocabulary for case in grouped):
            raise ValueError("Grouped cases must have identical candidate lists")
        pages = {}
        targets = []
        for case in grouped:
            targets.extend(case["input"]["targets"])
            for page in case["input"]["pages"]:
                page = {**page, "image": str((source / page["image"]).resolve())}
                if page["id"] in pages and page != pages[page["id"]]:
                    raise ValueError("Conflicting page IDs")
                pages[page["id"]] = page
        if len({t["id"] for t in targets}) != len(targets):
            raise ValueError("Duplicate target IDs")
        cases.append({"id": f"joint-{'present' if present else 'absent'}-{index}", "truth_id": truth,
                      "input": {"schema_version": 1, "pages": list(pages.values()), "targets": targets,
                                "candidates": vocabulary}})
    scripts = Path(__file__).resolve().parent / "adapters"
    adapters = []
    for upstream in original["adapters"]:
        adapter = json.loads(json.dumps(upstream))
        identity = adapter["id"]
        if identity == "local-single":
            adapter["id"] = "local-joint"
        elif identity in ("alex-gilbert-unredact", "mortenfj-unredactron"):
            adapter["id"] = identity + "-joint-adaptation"
            adapter["args"] = [str(scripts / "external_joint.py"), "--upstream",
                               "alex" if identity.startswith("alex") else "morten",
                               "--adapter", *adapter["args"][:1], "--", *adapter["args"][1:]]
            adapter["artifacts"].append(str(scripts / "external_joint.py"))
        else:
            continue
        adapters.append(adapter)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    suite["cases"] = cases
    suite["description"] = ("Joint adaptation track: identical same-name occurrences across three fonts; "
                            "our native solver vs OUR aggregation of unchanged external width measurements. "
                            "Supplied cross-document links, widths, sigma and vocabulary; no labels in requests.")
    (out / "suite.json").write_text(json.dumps(suite, indent=2) + "\n")
    manifest = {"schema_version": 1, "required_external": [a["id"] for a in adapters if a["kind"] == "external"],
                "adapters": adapters}
    (out / "adapters.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(out / "suite.json")


if __name__ == "__main__":
    main()
