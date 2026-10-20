# Copyright (C) 2026 Toit contributors.
"""Copy the blinded benchmark and repair only mosaic rendering coordinates.

The generator supplied page coordinates where the Rust scorer expects positions
inside the rounded target crop. This amendment uses mask geometry only, without
hidden glyph positions or labels. Existing frozen suites remain untouched.
"""
import argparse
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import shutil


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def correct(target):
    result = copy.deepcopy(target)
    evidence, bounds = result['evidence'], result['bounds']
    if evidence['kind'] != 'mosaic_auto':
        return result
    if evidence['origin_x'] != bounds['x'] or evidence['baseline_y'] != bounds['y'] + bounds['h'] - 2:
        raise ValueError('Input does not have the known generator coordinate error')
    evidence['origin_x'] -= math.floor(bounds['x'])
    evidence['baseline_y'] -= math.floor(bounds['y'])
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--input', type=Path, default=Path('runs/multidoc-blinded-001'))
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    parent = json.loads((a.input / 'manifest.json').read_text())
    for name, info in parent['suites'].items():
        if digest(a.input / (name + '.json')) != info['sha256']:
            raise ValueError('Parent suite changed')
    a.output.mkdir(parents=True, exist_ok=False)
    # Regular hardlinks preserve original image bytes without exposing descriptive
    # source names through symlink resolution. Small controller files are copied.
    for path in a.input.rglob('*'):
        if path.is_file() and path.suffix == '.pgm':
            dest = a.output / path.relative_to(a.input)
            dest.parent.mkdir(parents=True, exist_ok=True)
            os.link(path, dest)
    shutil.copytree(a.input / 'controller', a.output / 'controller', dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns('*.pgm'))
    manifest = copy.deepcopy(parent)
    changes, total = [], 0
    for name in sorted(parent['suites']):
        path = a.input / (name + '.json')
        suite = json.loads(path.read_text())
        for case in suite['cases']:
            total += 1
            before = copy.deepcopy(case)
            case['input']['targets'] = [correct(t) for t in before['input']['targets']]
            if case != before:
                reversed_case = copy.deepcopy(case)
                for new, old in zip(reversed_case['input']['targets'], before['input']['targets']):
                    new['evidence']['origin_x'] = old['evidence']['origin_x']
                    new['evidence']['baseline_y'] = old['evidence']['baseline_y']
                if reversed_case != before:
                    raise ValueError('Unexpected input change')
                changes.append(dict(case=case['id'], suite=name,
                    old=[t['evidence'] for t in before['input']['targets']],
                    new=[t['evidence'] for t in case['input']['targets']]))
        dest = a.output / path.name
        if any(c['suite'] == name for c in changes):
            dest.write_text(json.dumps(suite, indent=2) + '\n')
        else:
            shutil.copy2(path, dest)
        manifest['suites'][name]['sha256'] = digest(dest)
    if total != 3456 or len(changes) != 1152:
        raise ValueError('Expected 1152 mosaic amendments among3456 cases')
    amendment = dict(parent=str(a.input.resolve()), parent_manifest_sha256=digest(a.input / 'manifest.json'),
        script_sha256=digest(Path(__file__)), changed_cases=len(changes), total_cases=total,
        reason='Mosaic rendering origin/baseline are crop-relative, not page-relative. Subtract floor(crop origin); retain the original mask-only positioning estimate. No hidden glyph position, font, padding or baseline is supplied.',
        changes=changes)
    (a.output / 'coordinate-amendment.json').write_text(json.dumps(amendment, indent=2) + '\n')
    manifest['coordinate_amendment_sha256'] = digest(a.output / 'coordinate-amendment.json')
    (a.output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    files = [Path(__file__), a.output / 'manifest.json', a.output / 'coordinate-amendment.json',
             *a.output.glob('*.json'), *a.output.glob('controller/*.json')]
    frozen = {str(path.resolve()): digest(path) for path in files}
    (a.output / 'freeze.json').write_text(json.dumps(frozen, indent=2) + '\n')
    print(json.dumps(dict(cases=total, corrected=len(changes), output=str(a.output))))


if __name__ == '__main__':
    main()
