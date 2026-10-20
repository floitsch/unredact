# Copyright (C) 2026 Toit contributors.
"""Check incremental provider search against exhaustive scoring of the same vocabulary.

This is a protocol/search regression benchmark using a deterministic dictionary
provider. It does not evaluate an LLM or claim to reproduce another project.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/release/unredact"))
    parser.add_argument("--project", type=Path, required=True)
    parser.add_argument("--candidates", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--provider", type=Path, default=Path("examples/word_provider.py"))
    a = parser.parse_args()
    if a.output.exists():
        parser.error("Use a new output file")
    project = json.loads((a.project / "project.json").read_text())
    groups = sorted({t["group"] for t in project["targets"]})

    def run(*args):
        started = time.monotonic()
        result = subprocess.run([str(a.binary.resolve()), *map(str, args)],
                                check=True, capture_output=True, text=True)
        return json.loads(result.stdout), time.monotonic() - started

    def by_text(report):
        rows = {r["name"]: (r["evidence_score"], r["compatible"],
                             r["evidence_rank_min"], r["evidence_rank_max"])
                for r in report["candidates"]}
        if len(rows) != len(report["candidates"]):
            raise ValueError("This evaluator requires a vocabulary of distinct canonical texts")
        return rows

    cases = []
    for group in groups:
        direct, direct_time = run("solve", "--project", a.project, "--group", group,
                                  "--candidates", a.candidates, "--limit", "100000")
        search, search_time = run("search", "--project", a.project, "--group", group,
                                  "--provider", "python3", "--provider-arg", a.provider.resolve(),
                                  "--provider-arg", a.candidates.resolve(), "--beam-width", "256",
                                  "--max-extensions", "2048", "--max-completed", "50000",
                                  "--rounds", "32", "--limit", "100000")
        expected = by_text(direct)
        actual = by_text(search["result"]) if search["result"] else {}
        mismatches = []
        for name in sorted(set(expected) | set(actual)):
            e, r = expected.get(name), actual.get(name)
            if e is None or r is None or abs(e[0] - r[0]) > 1e-10 or e[1:] != r[1:]:
                mismatches.append(name)
        case = {
            "group": group,
            "direct_candidates": len(expected), "searched_candidates": len(actual),
            "score_compatibility_and_tie_mismatches": mismatches,
            "budget_truncated": search["budget_truncated"],
            "provider_exhaustive_each_round": all(r["provider_exhaustive_for_prefixes"] for r in search["rounds"]),
            "rounds": len(search["rounds"]),
            "direct_seconds": direct_time, "search_seconds": search_time,
        }
        cases.append(case)
        print(f"{group}: {len(actual)} candidates; {len(mismatches)} mismatches", flush=True)
    paths = {"binary": a.binary, "project": a.project / "project.json",
             "candidates": a.candidates, "provider": a.provider}
    report = {
        "scope": "Incremental word-provider search versus full scoring of the identical vocabulary; no AI or third-party accuracy claim",
        "sha256": {k: hashlib.sha256(v.read_bytes()).hexdigest() for k, v in paths.items()},
        "groups": len(cases),
        "passed": all(not c["score_compatibility_and_tie_mismatches"] and not c["budget_truncated"]
                      and c["provider_exhaustive_each_round"] for c in cases),
        "cases": cases,
    }
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(report, indent=2) + "\n")
    if not report["passed"]:
        raise SystemExit("Incremental search did not cover or match the complete comparison vocabulary")


if __name__ == "__main__":
    main()
