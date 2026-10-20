# Copyright (C) 2026 Toit contributors.
"""Join comparison evidence only when suite/input identities and repeated results agree."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--comparison',type=Path,action='append',required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    loaded=[(path,json.loads(path.read_text())) for path in a.comparison]
    if len({r['suite_sha256'] for _,r in loaded})!=1:
        raise ValueError('Different suites cannot be merged into one matched comparison')
    inputs,executions,rows,duplicate_checks={},{},{},[]
    for path,report in loaded:
        for e in report['executions']:
            case=e['case'];old=inputs.setdefault(case,e['input_sha256'])
            if old!=e['input_sha256']:raise ValueError(f'Input mismatch: {case}')
            key=(e['adapter'],case,e['repetition'])
            if key in executions:
                previous=executions[key]
                if previous['result']!=e['result'] or previous['metrics']!=e['metrics'] or previous['error']!=e['error']:
                    raise ValueError(f'Repeated configuration produced different results: {key}')
                duplicate_checks.append(dict(adapter=e['adapter'],case=case,repetition=e['repetition']))
            else:executions[key]=e
        for adapter,summary in report['summary'].items():
            if adapter in rows:
                # Timing is retained per source run, never pooled with overlapping cases.
                keys=set(summary)-{'mean_elapsed_seconds'}
                if any(rows[adapter]['summary'][k]!=summary[k] for k in keys):
                    raise ValueError(f'Configuration summary mismatch: {adapter}')
                rows[adapter]['runs'].append(dict(path=str(path),mean_elapsed_seconds=summary['mean_elapsed_seconds']))
            else:
                rows[adapter]=dict(summary=summary,runs=[dict(path=str(path),mean_elapsed_seconds=summary['mean_elapsed_seconds'])])
    coverage={adapter:{(e['case'],e['repetition']) for e in executions.values() if e['adapter']==adapter} for adapter in rows}
    if len({frozenset(c) for c in coverage.values()})!=1:
        raise ValueError('Configurations cover different cases/repetitions')
    output=dict(schema_version=1,suite_sha256=loaded[0][1]['suite_sha256'],
        sources=[dict(path=str(path),sha256=hashlib.sha256(path.read_bytes()).hexdigest()) for path,_ in loaded],
        case_input_sha256=inputs,configurations=rows,identical_repeated_results=duplicate_checks,
        interpretation='Matched inputs across pinned runs. Overlapping executions are consistency checks, not additional trials. Synthetic cases share words/fonts and are correlated. Catalog adaptations are configurations of upstream algorithms, not upstream defaults. No general original-redaction recovery claim.')
    with a.output.open('x') as out:json.dump(output,out,indent=2);out.write('\n')
    print('| Configuration | Correct unique first | Truth compatible | Empty set when truth absent | Failed |')
    print('| --- | --- | --- | --- | --- |')
    for adapter,row in rows.items():
        s=row['summary'];compatible=f"{s['truth_compatible']}/{s['truth_present_successes']}" if s['compatible_reported'] else 'not exposed'
        absent=f"{s['no_compatible_when_truth_absent']}/{s['truth_absent_successes']}" if s['compatible_reported'] else 'not exposed'
        print(f"| {adapter} | {s['unique_correct_first']}/{s['truth_present_successes']} | {compatible} | {absent} | {s['failed']} |")


if __name__=='__main__':main()
