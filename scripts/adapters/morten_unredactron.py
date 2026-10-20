# Copyright (C) 2026 Toit contributors.
"""Execute the pinned Unredactron profiler and its original candidate loop.

The image track supplies target boxes, so PDF loading and rectangle detection are
bypassed. The scoring loop is extracted verbatim as Python AST from analyze_pdf;
font fitting and candidate selection execute upstream code, without a reimplementation.
The wrapper records each width to expose score ties the upstream console hides.
"""
import argparse
import ast
import contextlib
import json
from pathlib import Path
import sys
from types import SimpleNamespace

from external_common import dump, groups, single_target, unsupported


def scoring_loop(path):
    module = ast.parse(path.read_text(), filename=str(path))
    function = next(n for n in module.body if isinstance(n, ast.FunctionDef) and n.name == "analyze_pdf")
    start = next(i for i, n in enumerate(function.body)
                 if isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "results" for t in n.targets))
    body = function.body[start:]
    if not isinstance(body[-1], ast.Return):
        raise ValueError("Upstream candidate loop changed; review adapter")
    wrapper = ast.parse("def score(font, scale_factor, tracking_offset, candidates, redactions, gray):\n pass\n")
    wrapper.body[0].body = body
    namespace = {}
    exec(compile(ast.fix_missing_locations(wrapper), str(path), "exec"), namespace)
    return namespace["score"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, required=True)
    args = parser.parse_args()
    request = json.load(sys.stdin)
    try:
        target, page, entries = single_target(request["input"])
    except ValueError as error:
        print(json.dumps(unsupported(str(error))))
        return
    sys.dont_write_bytecode = True
    repo = args.repository.resolve()
    sys.path.insert(0, str(repo))
    import numpy as np
    from PIL import Image, ImageFont
    from font_profiler import FontProfiler

    work = Path(request["work_dir"])
    work.mkdir(parents=True, exist_ok=False)
    pixels = np.array(Image.open(page["image"]).convert("RGB"))
    profiler = FontProfiler(str(repo / "fonts/fonts"))
    with contextlib.redirect_stdout(sys.stderr):
        profile = profiler.profile_document(pixels, dpi=page["dpi"], verbose=False)
    if profile is None:
        print(json.dumps(unsupported("Upstream profiler returned no usable profile")))
        return
    dump(work / "profile.json", profile.to_dict())
    font = ImageFont.truetype(profile.font_path, int(profile.font_size))
    measured = []

    class RecordingFont:
        def getlength(self, text):
            width = font.getlength(text)
            measured.append((text, width))
            return width

    evidence = target["evidence"]
    # Raw box width is the upstream default. Width evidence supports a separately
    # declared padding-corrected track without changing the scoring algorithm.
    width = evidence["width"] if evidence["kind"] == "width" else target["bounds"]["w"]
    box = target["bounds"]
    candidates = [{"name": text, "confidence": 0, "notes": ""} for _, text in entries]
    matches, _ = scoring_loop(repo / "unredactron.py")(
        RecordingFont(), profile.scale_factor, profile.tracking_offset, candidates,
        [(box["x"], box["y"], width, box["h"])], SimpleNamespace(shape=pixels.shape[:2]))
    if len(measured) != len(entries):
        raise ValueError("Upstream candidate enumeration changed; review adapter")
    scores = []
    details = []
    for (identity, text), (measured_text, base_width) in zip(entries, measured):
        if text != measured_text:
            raise ValueError("Upstream candidate order changed")
        predicted = base_width * profile.scale_factor + len(text) * profile.tracking_offset
        # Expose the exact upstream score (all supplied confidence values are zero).
        error = abs(predicted - width) / predicted * 100 if predicted > 0 else 100
        scores.append((identity, error))
        details.append({"id": identity, "text": text, "width": predicted, "score": error})
    ranked = groups(scores)
    best_error = min(s for _, s in scores)
    if best_error < 30:
        allowed = {text for identity, text in entries if identity in ranked[0]}
        if len(matches) != 1 or matches[0]["candidate"] not in allowed:
            raise ValueError("Extracted ranks disagree with upstream selection")
    elif matches:
        raise ValueError("Upstream acceptance behavior changed")
    dump(work / "native-result.json", {"matches": matches, "measurements": details,
                                      "target_width": width,
                                      "adaptation": "Supplied box; original candidate loop; neutral CSV confidence"})
    # Main upstream output exposes only its selected match, not a compatible set.
    print(json.dumps({"schema_version": 1, "status": "ok", "reason": None,
                      "ranked_groups": ranked, "ranking_complete": True, "compatible": None}))


if __name__ == "__main__":
    main()
