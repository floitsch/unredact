# Copyright (C) 2026 Toit contributors.
"""Declared joint extension of external per-occurrence width measurements.

This aggregation is OUR adaptation, not a feature of either upstream program.
Runs each pinned upstream adapter unchanged, then combines its raw predicted
widths using supplied measurement sigmas and padding intervals. Does not infer
font uncertainty, add AI scores, or use labels. Exact/explicit duplicates count
once. Raw baseline outputs and the aggregation details remain in the work dir.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
import sys

from external_common import dump, groups, unsupported


def observations(data):
    pages = {page["id"]: page for page in data["pages"]}
    seen_images = set()
    seen_dependencies = set()
    kept, ignored = [], []
    for target in data["targets"]:
        page = pages[target["page"]]
        box = target["bounds"]
        identity = (page["image_sha256"], *(round(box[k], 3) for k in ("x", "y", "w", "h")))
        dependency = target.get("dependence_key")
        duplicate = identity in seen_images or (dependency is not None and dependency in seen_dependencies)
        seen_images.add(identity)
        if dependency is not None:
            seen_dependencies.add(dependency)
        (ignored if duplicate else kept).append(target)
    return kept, ignored


def geometry(target):
    evidence = target["evidence"]
    if evidence["kind"] == "width":
        low = high = evidence["width"]
    elif evidence["kind"] == "opaque":
        low = target["bounds"]["w"] - evidence["padding_max"]
        high = target["bounds"]["w"] - evidence["padding_min"]
    else:
        raise ValueError("Joint width extension supports width and opaque evidence only")
    sigma = evidence["measurement_sigma"]
    if not all(math.isfinite(x) for x in (low, high, sigma)) or low <= 0 or high < low or sigma <= 0:
        raise ValueError("Invalid supplied width interval or measurement sigma")
    return low, high, sigma


def aggregate(data, widths):
    targets, ignored = observations(data)
    scores, compatible, details = [], [], []
    for candidate in data["candidates"]:
        cost, accepted, parts = 0., True, []
        for target in targets:
            surfaces = [s["text"] for s in candidate["surfaces"] if s["kind"] == target["surface_kind"]]
            if len(surfaces) != 1:
                raise ValueError("Requires exactly one matching surface per candidate")
            predicted = widths[target["id"]][surfaces[0]]
            if not math.isfinite(predicted) or predicted <= 0:
                raise ValueError("Invalid upstream width")
            low, high, sigma = geometry(target)
            error = max(low - predicted, predicted - high, 0.) / sigma
            part = .5 * error * error + math.log(sigma)
            cost += part
            accepted &= error <= 3
            parts.append({"target": target["id"], "predicted_width": predicted,
                          "interval": [low, high], "sigma": sigma, "standardized_error": error})
        scores.append((candidate["id"], cost))
        if accepted:
            compatible.append(candidate["id"])
        details.append({"id": candidate["id"], "score": cost, "observations": parts})
    return {"schema_version": 1, "status": "ok", "reason": None,
            "ranked_groups": groups(scores), "ranking_complete": True,
            "compatible": compatible}, {"candidates": details, "ignored_targets": [t["id"] for t in ignored]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", choices=["alex", "morten"], required=True)
    parser.add_argument("--adapter", type=Path, required=True)
    parser.add_argument("adapter_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    request = json.load(sys.stdin)
    data = request["input"]
    if not 1 <= len(data["targets"]) <= 20:
        print(json.dumps(unsupported("Joint extension supports 1..20 supplied occurrences")))
        return
    try:
        for target in data["targets"]:
            geometry(target)
    except ValueError as error:
        print(json.dumps(unsupported(str(error))))
        return
    work = Path(request["work_dir"])
    work.mkdir(parents=True, exist_ok=False)
    targets, _ = observations(data)
    widths = {}
    adapter_args = args.adapter_args[1:] if args.adapter_args[:1] == ["--"] else args.adapter_args
    for index, target in enumerate(targets):
        child_work = work / f"occurrence-{index}"
        child = {"schema_version": 1, "work_dir": str(child_work),
                 "input": {**data, "targets": [target],
                           "pages": [page for page in data["pages"] if page["id"] == target["page"]]}}
        dump(work / f"request-{index}.json", child)
        proc = subprocess.run([sys.executable, str(args.adapter), *adapter_args],
                              input=json.dumps(child), text=True, capture_output=True, timeout=120)
        (work / f"stderr-{index}.txt").write_text(proc.stderr)
        if proc.returncode:
            raise RuntimeError(f"Upstream adapter failed at {target['id']}: {proc.stderr[-4000:]}")
        response = json.loads(proc.stdout)
        dump(work / f"response-{index}.json", response)
        if response["status"] != "ok":
            print(json.dumps({**unsupported(f"Occurrence {target['id']}: {response['reason']}"),
                              "status": response["status"]}))
            return
        native = json.loads((child_work / "native-result.json").read_text())
        if args.upstream == "alex":
            widths[target["id"]] = dict(native["measurements"])
        else:
            widths[target["id"]] = {item["text"]: item["width"] for item in native["measurements"]}
    response, detail = aggregate(data, widths)
    detail["adaptation"] = "Our joint width-cost aggregation of unchanged upstream per-occurrence measurements"
    detail["uncertainty"] = "Uses supplied measurement sigma only; upstream model uncertainty unavailable"
    dump(work / "joint-result.json", detail)
    print(json.dumps(response))


if __name__ == "__main__":
    main()
