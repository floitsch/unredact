# Copyright (C) 2026 Toit contributors.
"""Declared font-directory control; Unredactron profiling/scoring stay unchanged."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
import morten_unredactron


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--font-catalog', type=Path, required=True)
    parser.add_argument('--repository', type=Path, required=True)
    args = parser.parse_args()
    raw = sys.stdin.read()
    request = json.loads(raw)
    work = Path(request['work_dir'])
    fonts = work.with_name(work.name + '-catalog-fonts')
    fonts.mkdir(parents=True, exist_ok=False)
    entries = json.loads(args.font_catalog.read_text())
    if not isinstance(entries, list) or not 1 <= len(entries) <= 1000:
        raise ValueError('Catalog must contain 1..1000 entries')
    sources = []
    seen = set()
    filenames = set()
    for entry in entries:
        path = (args.font_catalog.parent / entry['path']).resolve()
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        if digest != entry['sha256']:
            raise ValueError('Catalog font hash mismatch')
        if digest in seen:
            continue
        seen.add(digest)
        filename = path.name
        if path.suffix.lower() != '.ttf' or filename in filenames:
            raise ValueError('Upstream directory control requires distinct TTF basenames')
        filenames.add(filename)
        (fonts / filename).write_bytes(data)
        sources.append(dict(file=filename, id=entry['id'], source=str(path), sha256=digest))
    (fonts / 'provenance.json').write_text(json.dumps(dict(
        adaptation='Our supplied font directory; upstream profiler/scoring unchanged',
        catalog_sha256=hashlib.sha256(args.font_catalog.read_bytes()).hexdigest(), fonts=sources), indent=2))
    sys.path.insert(0,str(args.repository.resolve()))
    import font_profiler
    original = font_profiler.FontProfiler
    font_profiler.FontProfiler = lambda fonts_dir: original(str(fonts))
    sys.stdin = io.StringIO(raw)
    sys.argv = [sys.argv[0], '--repository', str(args.repository)]
    morten_unredactron.main()


if __name__ == '__main__':main()
