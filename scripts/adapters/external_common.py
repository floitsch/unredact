# Copyright (C) 2026 Toit contributors.
"""Small protocol helpers; no typography or identity inference."""
import json
import math


def unsupported(reason):
    return {"schema_version": 1, "status": "unsupported", "reason": reason,
            "ranked_groups": [], "ranking_complete": False, "compatible": None}


def single_target(data):
    if len(data["targets"]) != 1:
        raise ValueError("Upstream analyzes one occurrence; use the single-occurrence suite")
    target = data["targets"][0]
    if target["evidence"]["kind"] not in ("opaque", "width"):
        raise ValueError("Upstream width analysis does not support this evidence type")
    entries = []
    for candidate in data["candidates"]:
        surfaces = [s["text"] for s in candidate["surfaces"] if s["kind"] == target["surface_kind"]]
        if len(surfaces) != 1:
            raise ValueError("This adapter requires one matching surface per candidate")
        entries.append((candidate["id"], surfaces[0]))
    page = next(p for p in data["pages"] if p["id"] == target["page"])
    return target, page, entries


def groups(scores):
    """Group numerical ties, preserving an explicit physical ambiguity."""
    ordered = sorted(scores, key=lambda item: (item[1], item[0]))
    result = []
    previous = None
    for identity, score in ordered:
        if not math.isfinite(score):
            raise ValueError("Non-finite baseline score")
        if previous is None or abs(previous - score) > 1e-9:
            result.append([])
            previous = score
        result[-1].append(identity)
    return result


def dump(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")
