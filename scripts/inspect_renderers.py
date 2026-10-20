# Copyright (C) 2026 Toit contributors.
"""Record installed renderer releases, shared-library identities, and source pins."""
import argparse
import ctypes
import ctypes.util
import hashlib
import json
from pathlib import Path
import subprocess


def sha(path):
    with Path(path).open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source-root', type=Path, default=Path('runs/renderer-sources'))
    args = parser.parse_args()
    releases = {}
    for name, symbol in [('harfbuzz', 'hb_version_string'), ('cairo', 'cairo_version_string'),
                         ('poppler-glib', 'poppler_get_version')]:
        lib = ctypes.CDLL(ctypes.util.find_library(name))
        function = getattr(lib, symbol)
        function.restype = ctypes.c_char_p
        releases[name] = function().decode()
    ft = ctypes.CDLL(ctypes.util.find_library('freetype'))
    library = ctypes.c_void_p()
    assert ft.FT_Init_FreeType(ctypes.byref(library)) == 0
    major, minor, patch = ctypes.c_int(), ctypes.c_int(), ctypes.c_int()
    ft.FT_Library_Version(library, ctypes.byref(major), ctypes.byref(minor), ctypes.byref(patch))
    ft.FT_Done_FreeType(library)
    releases['freetype'] = f'{major.value}.{minor.value}.{patch.value}'
    paths = set()
    for line in Path('/proc/self/maps').read_text().splitlines():
        parts = line.split()
        if len(parts) >= 6:
            path = Path(parts[-1])
            if any(path.name.startswith('lib'+name) for name in ['harfbuzz', 'freetype', 'cairo', 'poppler']):
                paths.add(path)
    sources = []
    for directory in sorted(args.source_root.iterdir()):
        if not (directory/'.git').is_dir():
            continue
        sources.append(dict(path=str(directory.resolve()),
            remote=command('git', '-C', str(directory), 'remote', 'get-url', 'origin'),
            commit=command('git', '-C', str(directory), 'rev-parse', 'HEAD'),
            tag=command('git', '-C', str(directory), 'describe', '--tags', '--exact-match'),
            dirty=bool(command('git', '-C', str(directory), 'status', '--porcelain'))))
    fonts = sorted({command('fc-match', '-f', '%{file}', name) for name in
                    ['Liberation Serif','Liberation Sans','Liberation Mono','DejaVu Serif','DejaVu Sans']})
    result = dict(releases=releases,
        loaded_libraries=[dict(path=str(p), sha256=sha(p)) for p in sorted(paths)],
        sources=sources, default_fonts=[dict(path=p,sha256=sha(p)) for p in fonts],
        bridge_sha256=sha('native/bridge.c'),
        limitations=['Source tags match installed release numbers; installed binaries were not rebuilt from these checkouts.',
                     'This is a renderer inventory, not an assertion that it matches any document producer.',
                     'Source version alone does not fix font files, hinting, glyph rounding, line layout, or scan degradation.'])
    args.output.parent.mkdir(parents=True,exist_ok=True)
    with args.output.open('x') as file:
        json.dump(result,file,indent=2)
        file.write('\n')
    print(json.dumps(releases))


if __name__ == '__main__':
    main()
