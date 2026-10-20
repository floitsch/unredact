# Copyright (C) 2026 Toit contributors.
"""Declared font-catalog adaptation of the pinned Alex browser adapter.

Only the served font manifest/assets change; upstream fitting and scoring code
remain unchanged. This is not the upstream application's default configuration.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import alex_browser


def install_catalog(site, catalog_path):
    catalog_path = Path(catalog_path)
    entries = json.loads(catalog_path.read_text())
    if not isinstance(entries, list) or not 1 <= len(entries) <= 1000:
        raise ValueError('Catalog must contain 1..1000 entries')
    fonts, sources, seen = [], [], set()
    for entry in entries:
        path = (catalog_path.parent / entry['path']).resolve()
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        if digest != entry['sha256']:
            raise ValueError('Catalog font hash mismatch')
        if digest in seen:
            continue
        seen.add(digest)
        name = 'catalog-' + digest
        # Browser font sniffing supports the exact TTF/OTF bytes. A stable safe
        # filename and family alias avoid injecting catalog text into JS or CSS.
        filename = name + '.font'
        (site / 'fonts' / filename).write_bytes(data)
        fonts.append(dict(id=name, name=name, file=filename))
        sources.append(dict(alias=name, id=entry['id'], source=str(path), sha256=digest))
    (site / 'fonts' / 'manifest.json').write_text(json.dumps(dict(fonts=fonts), indent=2))
    (site.parent / 'catalog-adaptation.json').write_text(json.dumps(dict(
        adaptation='Our replacement font assets/manifest; upstream fitting/scoring unchanged',
        catalog_sha256=hashlib.sha256(catalog_path.read_bytes()).hexdigest(), fonts=sources), indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--font-catalog', type=Path, required=True)
    args, remaining = parser.parse_known_args()
    prepare = alex_browser.prepare_site

    def adapted(repo, pkg, vendor, site, image):
        prepare(repo, pkg, vendor, site, image)
        install_catalog(site, args.font_catalog)

    alex_browser.prepare_site = adapted
    sys.argv = [sys.argv[0], *remaining]
    alex_browser.main()


if __name__ == '__main__':
    main()
