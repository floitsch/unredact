# Copyright (C) 2026 Toit contributors.
"""Registered equal-publisher-family sensitivity analysis of complete test runs.

Preserves primary reports. Failures remain in planned denominators; an interface
that never reports a decision remains unavailable. Conditions are analyzed
separately, not counted as independent extra documents.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import statistics

from summarize_multidoc_benchmark import cluster_interval


def load(path): return json.loads(Path(path).read_text())
def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def family_analysis(records, families, tracks, conditions, primary, cases_per_document=24, seed=41762, draws=10000):
    """Only supplied-answer records, with exact planned coverage required."""
    documents=set(families)
    expected=len(documents)*cases_per_document
    buckets=defaultdict(list)
    seen=set()
    for record in records:
        if record['document'] not in documents or record['track'] not in tracks or record['condition'] not in conditions:
            raise ValueError('Unexpected document, method or condition')
        identity=(record['condition'],record['track'],record['case'])
        if identity in seen:raise ValueError('Duplicate case/track/condition')
        seen.add(identity);buckets[record['condition'],record['track']].append(record)
    if set(buckets)!={(c,t) for c in conditions for t in tracks}:
        raise ValueError('Missing planned method or condition')
    metrics=[];paired=[];means={}
    for (condition,track),rows in sorted(buckets.items()):
        if len(rows)!=expected or Counter(r['document'] for r in rows)!={d:cases_per_document for d in documents}:
            raise ValueError('Incomplete planned document/case coverage')
        by_family=defaultdict(list)
        for row in rows:by_family[families[row['document']]].append(row)
        available=all(row['status']!='not_available' for row in rows)
        values={family:statistics.mean(float(r['correct_first']) for r in subset)
                for family,subset in sorted(by_family.items())} if available else None
        means[condition,track]=values
        metrics.append(dict(condition=condition,track=track,cases=len(rows),documents=len(documents),families=len(by_family),
            statuses=dict(Counter(r['status'] for r in rows)),
            family_cases={k:len(v) for k,v in sorted(by_family.items())},
            family_means=values,family_macro_first=statistics.mean(values.values()) if values else None,
            family_95_interval=cluster_interval(list(values.values()),seed=seed,draws=draws) if values else None))
    for condition in sorted(conditions):
        for other in sorted(set(tracks)-{primary}):
            a,b=means[condition,primary],means[condition,other]
            if a is None or b is None:
                paired.append(dict(condition=condition,first=primary,second=other,status='not_available',
                                   reason='One method does not expose this comparison; no families omitted'))
                continue
            differences=[a[f]-b[f] for f in sorted(a)]
            paired.append(dict(condition=condition,first=primary,second=other,status='ok',families=len(differences),
                family_macro_difference=statistics.mean(differences),
                family_95_interval=cluster_interval(differences,seed=seed,draws=draws)))
    return dict(metrics=metrics,paired=paired)


def normalize_geometry(report):
    result=[]
    for row in report['rows']:
        if row['split']!='test':raise ValueError('Development rows in held-out report')
        if row['availability']=='present':
            result.append(dict(case=row['case'],document=row['document'],condition=row['condition'],track=row['adapter'],
                status=row['status'],correct_first=bool(row['metrics'].get('unique_correct_first',False)) if row['status']=='ok' else False))
    return result


def normalize_language(report):
    result=[]
    for row in report['rows']:
        if row['availability']!='present':continue
        for track,value in row['tracks'].items():
            result.append(dict(case=row['case'],document=row['document'],condition='opaque',track=track,
                status=value['status'],correct_first=value.get('unique_correct_first',False) if value['status']=='ok' else False))
    return result


def normalize_pixelation(report):
    result=[]
    for row in report['rows']:
        if row['availability']!='present':continue
        for track,value in row['engines'].items():
            result.append(dict(case=row['case'],document=row['document'],condition=row['condition'],track=track,
                status=value['status'],correct_first=value.get('truth_rank')==[1,1] if value['status']=='ok' else False))
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan',type=Path,default=Path('runs/heldout-analysis-001/plan.json'))
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();plan=load(a.plan)
    for path,sha in load(a.plan.parent/'freeze.json').items():
        if digest(path)!=sha:raise ValueError('Registered analysis plan changed')
    opaque,pixel=map(Path,plan['sources'])
    paths=dict(geometry=opaque/'geometry-summary.json',language=opaque/'language-evaluation/summary.json',pixelation=pixel/'evaluation/summary.json')
    reports={name:load(path) for name,path in paths.items()}  # All full reports required.
    for name,expected in [('language',384),('pixelation',768)]:
        if len(reports[name]['rows'])!=expected or not reports[name]['all_frozen_hashes_verified']:
            raise ValueError('Incomplete or unverified source report')
    if len(reports['geometry']['rows'])!=1152:raise ValueError('Incomplete geometry report')
    for evaluation in [opaque/'language-evaluation',pixel/'evaluation']:
        for path,sha in load(evaluation/'freeze.json')['files'].items():
            if digest(path)!=sha:raise ValueError(f'Frozen experiment dependency changed:{path}')
    common=dict(families=plan['document_families'],seed=plan['bootstrap_seed'],draws=plan['bootstrap_draws'])
    geometry_tracks=load(opaque/'plan.json')['adapters']
    language_tracks=['language-only','local-profile-cost-weight-0','local-profile-cost-weight-1',
                     'alex-matched-catalog-wide-adaptation','morten-matched-catalog-adaptation']
    result=dict(geometry=family_analysis(normalize_geometry(reports['geometry']),tracks=geometry_tracks,conditions=['opaque'],primary='local-profile-cost',**common),
        language=family_analysis(normalize_language(reports['language']),tracks=language_tracks,conditions=['opaque'],primary=plan['opaque_primary'],**common),
        pixelation=family_analysis(normalize_pixelation(reports['pixelation']),tracks=['legacy','unit','shared_cells'],conditions=['pixelate-8','pixelate-16'],primary=plan['pixelation_primary'],**common))
    output=dict(schema_version=1,plan=plan,plan_sha256=digest(a.plan),script_sha256=digest(Path(__file__)),
        sources={name:dict(path=str(path.resolve()),sha256=digest(path)) for name,path in paths.items()},
        **result,interpretation='Secondary registered sensitivity: each publisher family receives equal weight. Conditions remain separate; all planned failures retained. Five sampled families do not represent all document producers. Original case/document estimates are unchanged. Missing compatibility APIs remain unavailable. Public-document pretrained exposure is unknown.')
    with a.output.open('x') as f:json.dump(output,f,indent=2);f.write('\n')
    for domain in result:
        for row in result[domain]['paired']:print(domain,row)


if __name__=='__main__':main()
