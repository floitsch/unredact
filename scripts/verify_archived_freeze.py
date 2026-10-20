# Copyright (C) 2026 Toit contributors.
"""Verify historical experiment bytes using exact archived snapshots after integration."""
import argparse
import hashlib
import json
from pathlib import Path


def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('freeze',type=Path)
    p.add_argument('--snapshots',type=Path,action='append',default=[])
    a=p.parse_args();data=json.loads(a.freeze.read_text());files=data.get('files',data)
    maps=[json.loads(path.read_text()) for path in a.snapshots]
    current=archived=0
    for name,expected in files.items():
        if not isinstance(expected,str):raise ValueError('Expected path-to-SHA256 file map')
        path=Path(name)
        if path.is_file() and digest(path)==expected:current+=1;continue
        choices=[m[name] for m in maps if name in m]
        valid=[c for c in choices if c['sha256']==expected and digest(c['snapshot'])==expected]
        if not valid:raise ValueError(f'No original or archived bytes match:{name}')
        archived+=1
    print(json.dumps(dict(freeze=str(a.freeze),verified=current+archived,current=current,archived=archived)))


if __name__=='__main__':main()
