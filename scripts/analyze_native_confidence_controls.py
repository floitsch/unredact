# Copyright (C) 2026 Toit contributors.
"""Compare every declared native-confidence control with the fixed primary.

This supplementary analysis uses paired document and publisher-family clusters.
It never chooses a confidence scale, edits a prediction or calls a model.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path

from analyze_heldout_families import family_analysis


def load(path):return json.loads(Path(path).read_text())
def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def records(language, native, primary, methods, families, spans):
    """Keep failures and identical planned coverage across all methods."""
    def index(rows):
        indexed={r['case']:r for r in rows}
        if len(indexed)!=len(rows) or len(rows)!=2*spans:raise ValueError('Full unique case coverage required')
        if Counter(r['availability'] for r in rows)!={'present':spans,'absent':spans}:
            raise ValueError('Both planned availability controls required')
        return indexed
    languages=index(language['rows']);natives=index(native['rows'])
    if languages.keys()!=natives.keys():raise ValueError('Language and native cases differ')
    result=[]
    for identity,row in languages.items():
        other=natives[identity]
        if (row['document'],row['availability'])!=(other['document'],other['availability']):
            raise ValueError('Paired document/availability metadata differs')
        if row['document'] not in families:raise ValueError('Unexpected document')
        if row['availability']!='present':continue
        if other['status']=='ok' and (set(other['variants'])!=set(methods) or not other['zero_replay_exact']):
            raise ValueError('Native variants differ or neutral replay is unverified')
        base=dict(case=identity,document=row['document'],condition='opaque')
        actual=row['tracks'][primary]
        result.append(dict(**base,track=primary,status=actual['status'],
            correct_first=actual['status']=='ok' and actual['unique_correct_first']))
        for method in methods:
            result.append(dict(**base,track=method,status=other['status'],
                correct_first=other['status']=='ok' and other['variants'][method]['selected_correct']))
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan)
    for path,sha in load(a.plan.parent/'freeze.json').items():
        if digest(path)!=sha:raise ValueError('Registered supplementary analysis changed:'+path)
    language_path=Path(plan['language_analysis']);native_path=Path(plan['native_controls'])
    language,native=load(language_path),load(native_path)
    if not language['all_frozen_hashes_verified']:raise ValueError('Audited language run required')
    for report,field in [(language,'source_files'),(native,'files')]:
        for path,sha in report[field].items():
            if digest(path)!=sha:raise ValueError('Comparison source changed:'+path)
    native_plan=load(plan['native_plan'])
    if native['plan']!=native_plan or native_plan['confidence_scales']!=plan['confidence_scales']:
        raise ValueError('Native source protocol/scales changed')
    methods=['native-zero',*[f't5-confidence-scale-{s:g}' for s in plan['confidence_scales']]]
    rows=records(language,native,plan['primary'],methods,plan['document_families'],plan['spans'])
    tracks=[plan['primary'],*methods]
    common=dict(tracks=tracks,conditions=['opaque'],primary=plan['primary'],
        cases_per_document=plan['cases_per_document'],seed=plan['bootstrap_seed'],draws=plan['bootstrap_draws'])
    by_document=family_analysis(rows,families={d:d for d in plan['document_families']},**common)
    by_family=family_analysis(rows,families=plan['document_families'],**common)
    result=dict(plan=plan,rows=rows,document_analysis=by_document,publisher_family_analysis=by_family,
        source_files={str(path.resolve()):digest(path) for path in [a.plan,Path(__file__),language_path,native_path,Path(plan['native_plan'])]},
        interpretation='Supplementary comparison through native CSV-confidence hook, not native automatic AI. All fixed scales reported. Both methods use identical cached T5 sequence scores; zero replay checks actual pinned native loop. Primary uses unique-first accuracy; native selected-match accuracy credits its own tie selection. Operational failures remain incorrect; missing native language context falls back to zero confidence. Document/family intervals are paired and descriptive; no confidence-scale selection or calibrated identity claim.')
    with a.output.open('x') as f:json.dump(result,f,indent=2);f.write('\n')
    for label,analysis in [('document',by_document),('publisher_family',by_family)]:
        for row in analysis['paired']:print(label,json.dumps(row))


if __name__=='__main__':main()
