# Copyright (C) 2026 Toit contributors.
"""Fetch a declared public PDF source list, preserving hashes and failures.

Completed matching files are reused only after their saved hashes are verified.
Changing the source list is an explicit plan change, never automatic fallback.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import subprocess
from urllib.parse import urlparse


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--workers',type=int,default=2)
    a=p.parse_args()
    if not 1 <= a.workers <= 8:p.error('workers must be 1..8')
    plan=json.loads(a.plan.read_text());a.output.mkdir(parents=True,exist_ok=True)
    manifest=a.output/'sources.json'
    old={r['id']:r for r in json.loads(manifest.read_text())} if manifest.exists() else {}
    documents=plan['documents']
    if len({s['id'] for s in documents})!=len(documents):raise ValueError('Duplicate source ID')
    for s in documents:
        if not s['id'] or any(c not in 'abcdefghijklmnopqrstuvwxyz0123456789-_' for c in s['id']):
            raise ValueError('Unsafe source ID')
        if urlparse(s['url']).scheme!='https':raise ValueError('Sources must use HTTPS')
    def fetch(spec):
        destination=a.output/(spec['id']+'.pdf');previous=old.get(spec['id'])
        if previous and not previous.get('error') and previous['url']==spec['url'] and destination.exists():
            if digest(destination)!=previous['sha256']:raise ValueError('Cached source changed')
            return {**previous,**spec,'path':str(destination.resolve())}
        temp=destination.with_suffix('.pdf.partial')
        result=subprocess.run(['curl','--fail','--location','--proto','=https','--proto-redir','=https',
            '--retry','1','--max-time','600','--max-filesize','314572800','--silent','--show-error',
            spec['url'],'--output',str(temp)],capture_output=True,text=True,timeout=1250)
        row=dict(**spec,path=str(destination.resolve()),returncode=result.returncode,
                 error=result.stderr if result.returncode else None,download_diagnostics=result.stderr)
        if result.returncode==0:
            if temp.read_bytes()[:5]!=b'%PDF-':row['error']='Response was not a PDF'
            else:
                check=subprocess.run(['pdfinfo',str(temp)],capture_output=True,text=True,timeout=30)
                if check.returncode:row['error']='pdfinfo rejected download: '+check.stderr
                else:
                    temp.replace(destination);row.update(sha256=digest(destination),bytes=destination.stat().st_size)
        if row['error']:temp.unlink(missing_ok=True)
        print(spec['id'],row['error'] or 'downloaded',flush=True)
        return row
    with ThreadPoolExecutor(max_workers=a.workers) as executor:rows=list(executor.map(fetch,documents))
    temp=manifest.with_suffix('.json.tmp');temp.write_text(json.dumps(rows,indent=2)+'\n');temp.replace(manifest)
    if any(r['error'] for r in rows):raise SystemExit('Source errors recorded; no replacements selected')


if __name__=='__main__':main()
