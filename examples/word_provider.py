# Copyright (C) 2026 Toit contributors.
"""Deterministic reference implementation of the incremental provider protocol.

Pass a vocabulary JSON/text path as the first argument. This is a dictionary
provider, not an AI model. A model adapter should return the same reply schema.
"""
import json
import re
import sys
from pathlib import Path

request = json.load(sys.stdin)
if request.get("schema_version") != 2 or request.get("task") != "extend_prefixes":
    raise ValueError("Expected incremental search protocol version 2")
path = Path(sys.argv[1])
kind = request["surface_kind"]
if path.suffix == ".json":
    data = json.loads(path.read_text())
    vocabulary = {
        surface["text"]
        for candidate in data["candidates"]
        for surface in candidate["surfaces"]
        if surface.get("kind", "canonical") == kind
    }
else:
    vocabulary = {line.strip() for line in path.read_text().splitlines() if line.strip()}
extensions = []
for parent in request["prefixes"]:
    branches = {}
    for text in sorted(vocabulary):
        if not text.startswith(parent["text"]):
            continue
        remaining = text[len(parent["text"]):]
        if remaining:
            append = re.match(r"\s*\S+\s*", remaining).group(0)
            complete = len(append) == len(remaining)
        else:
            append, complete = "", True
        branches[(append, complete)] = branches.get((append, complete), 0) + 1
    for (append, complete), count in sorted(branches.items()):
        extensions.append({"parent_id": parent["id"], "append": append,
                           "complete": complete, "context_score": float(count)})
limit = request["limits"]["max_extensions"]
json.dump({"extensions": extensions[:limit],
           "exhaustive_for_prefixes": len(extensions) <= limit}, sys.stdout)
