# Copyright (C) 2026 Toit contributors.
"""Download a pinned public model; verify every artifact before recording it."""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.request

REPOSITORY = 'google-bert/bert-base-uncased'
REVISION = '86b5e0934494bd15c9632b12f734a8a67f723594'
EXPECTED = {
    "config.json": "7160e1553ad2ca51d8c1cb066be533db31826e12d173824c1bb0cb1a4f187d20",
    "tokenizer.json": "ce64fce797c24f68df90b40a3f74f579b336a493db14bd583fd520ea0d8c9a98",
    "tokenizer_config.json": "a025160ef0431f1a392f6f050c1310f4c5d9fb6f275932dbccba73c4d214bf10",
    "vocab.txt": "07eced375cec144d27c900241f3e339478dec958f92fddbc551f295c992038a3",
    "LICENSE": "43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1",
    "README.md": "9187b6018ea0010d884e78e098e328faa1b88b301570d0cce606bb35e4067e17",
    "model.safetensors": "68d45e234eb4a928074dfd868cead0219ab85354cc53d20e772753c6bb9169d3"
}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main(repository=REPOSITORY, revision=REVISION, expected_hashes=EXPECTED):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    files = {}
    for name, expected in expected_hashes.items():
        path = args.output / name
        url = f'https://huggingface.co/{repository}/resolve/{revision}/{name}'
        if not path.exists():
            partial = path.with_suffix(path.suffix + '.partial')
            with urllib.request.urlopen(url, timeout=120) as response, partial.open('xb') as stream:
                while chunk := response.read(1024 * 1024):
                    stream.write(chunk)
            if digest(partial) != expected:
                raise ValueError(f'Downloaded artifact hash mismatch: {name}; retained .partial for inspection')
            partial.rename(path)
        if digest(path) != expected:
            raise ValueError(f'Existing artifact hash mismatch: {name}')
        files[name] = dict(sha256=expected, bytes=path.stat().st_size, url=url)
        print(f'Verified {name}', flush=True)
    provenance = dict(repository=repository, revision=revision, files=files)
    path = args.output / 'provenance.json'
    if path.exists():
        if json.loads(path.read_text()) != provenance:
            raise ValueError('Existing provenance differs; refusing to overwrite')
    else:
        path.write_text(json.dumps(provenance, indent=2) + '\n')


if __name__ == '__main__':
    main()
