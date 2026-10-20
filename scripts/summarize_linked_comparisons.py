# Copyright (C) 2026 Toit contributors.
"""Summarize completed linked-scan comparisons and audit evidence aggregation."""
import argparse
import json
from pathlib import Path


def load(path):
    return json.loads(path.read_text())


def metrics(rows):
    good = [r for r in rows if not r['error'] and r['result']['status'] == 'ok']
    present = [r['metrics'] for r in good if r['metrics']['truth_in_vocabulary']]
    absent = [r['metrics'] for r in good if not r['metrics']['truth_in_vocabulary']]
    return dict(present=sum('-present-' in r['case'] for r in rows),
        absent=sum('-absent-' in r['case'] for r in rows),
        successful_present=len(present), successful_absent=len(absent),
        failures=[dict(case=r['case'], error=r['error'], result=r['result']) for r in rows if r not in good],
        unique_first=sum(r['unique_correct_first'] for r in present),
        best_tied=sum(r['truth_in_best_group'] for r in present),
        truth_compatible=sum(r['truth_compatible'] for r in present),
        mean_compatible_on_success=sum(r['compatible_candidates'] for r in present)/len(present) if present else None,
        empty_when_absent=sum(r['no_compatible'] for r in absent),
        single_wrong_when_absent=sum(r['single_wrong_compatible'] for r in absent))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--local', type=Path, required=True)
    p.add_argument('--external', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    local, external = load(a.local/'comparison.json'), load(a.external/'comparison.json')
    adapters = sorted(local['summary'])
    expected = {(c, adapter) for c in {r['case'] for r in local['executions']} for adapter in adapters}
    entries = {}
    for file in a.local.glob('*/execution.json'):
        r = load(file)
        key = (r['case'], r['adapter'])
        if key in entries:
            raise ValueError('Repeated executions must not count as independent cases')
        entries[key] = (r, file.parent/'work')
    if set(entries) != expected or len(expected) != 108:
        raise ValueError('Expected complete 36-case, three-adapter local coverage')
    conditions = {}
    for adapter in adapters:
        for condition in ['first', 'second', 'joint']:
            rows = [r for (case, tool), (r, _) in entries.items()
                    if tool == adapter and case.startswith(condition+'-')]
            conditions[condition+'::'+adapter] = metrics(rows)
    external_rows = external['executions']
    if len(external_rows) != 24:
        raise ValueError('Expected 12 joint cases for each of two external adaptations')
    external_keys = {(r['case'], r['adapter']) for r in external_rows}
    expected_external = {(case, adapter) for case, _ in expected if case.startswith('joint-')
                         for adapter in external['summary']}
    if external_keys != expected_external or len(external_keys) != len(external_rows):
        raise ValueError('External coverage differs or contains repetitions')
    for row in external_rows:
        if row['input_sha256'] != entries[(row['case'], adapters[0])][0]['input_sha256']:
            raise ValueError('External joint case used different input')
    for adapter in sorted(external['summary']):
        conditions['joint::'+adapter] = metrics([r for r in external_rows if r['adapter'] == adapter])
    # Same image, fonts and calibration should give the same physical report in
    # all local language modes. Linked independent page costs must add once.
    verified_modes, failed_modes = [], []
    for case in sorted({r['case'] for r in local['executions']}):
        ok = [not entries[(case, adapter)][0]['error'] and
              entries[(case, adapter)][0]['result']['status'] == 'ok' for adapter in adapters]
        if not all(ok):
            if any(ok):
                raise ValueError('Only some local modes completed for identical physical input')
            failed_modes.append(case)
            continue
        reports = [load(entries[(case, adapter)][1]/'native-result.json') for adapter in adapters]
        if any(r != reports[0] for r in reports[1:]):
            raise ValueError('Physical report changed between language modes')
        verified_modes.append(case)
    pair_rows = []
    width = 'width-fringe-default-filtered'
    for i in range(6):
        pair = dict(pair=f'pair-{i:03}', presence={})
        for presence in ['present', 'absent']:
            names = [f'{part}-{presence}-pair-{i:03}' for part in ['first', 'second', 'joint']]
            failed = [name for name in names if name in failed_modes]
            if failed:
                pair['presence'][presence] = dict(audit_status='unavailable', failed_cases=failed)
                continue
            reports = [load(entries[(name, width)][1]/'native-result.json') for name in names]
            by_id = [{c['id']: c for c in r['candidates']} for r in reports]
            allowed = [{c['id'] for c in r['candidates'] if c['compatible']} for r in reports]
            if allowed[2] != allowed[0] & allowed[1]:
                raise ValueError('Joint compatibility is not the intersection for independent page profiles')
            max_delta = max(abs(c['evidence_score']-by_id[0][cid]['evidence_score']-by_id[1][cid]['evidence_score'])
                            for cid,c in by_id[2].items())
            if max_delta > 1e-8 or [r['independent_observations'] for r in reports] != [1,1,2]:
                raise ValueError('Linked physical evidence did not add exactly once')
            detail = dict(audit_status='verified', compatible_counts=[len(s) for s in allowed],
                joint_reduces_beyond_better_single=len(allowed[2]) < min(len(allowed[0]), len(allowed[1])),
                max_physical_cost_delta=max_delta, language={})
            for adapter in adapters:
                if adapter == width:
                    continue
                contextual = [load(entries[(name, adapter)][1]/'context-result.json') for name in names]
                joint = contextual[2]
                merged_context = {**contextual[0]['contexts'], **contextual[1]['contexts']}
                if joint['contexts'] != merged_context:
                    raise ValueError('Linking changed visible context')
                single_scores = [{r['id']: r['mean_sequence_log_score'] for r in c['ranked']} for c in contextual[:2]]
                delta = max((abs(r['mean_sequence_log_score']-(single_scores[0][r['id']]+single_scores[1][r['id']])/2)
                             for r in joint['ranked']), default=0.)
                if delta > 1e-4:
                    raise ValueError('Joint language score differs from mean of independent occurrences')
                detail['language'][adapter] = dict(max_mean_score_delta=delta,
                    truth_ranks=[entries[(name,adapter)][0]['metrics']['truth_rank'] for name in names])
            pair['presence'][presence] = detail
        pair_rows.append(pair)
    output = dict(conditions=conditions,pairs=pair_rows,
        audits=dict(local_executions=108,external_joint_executions=24,
                    physical_reports_equal_across_modes_for_cases=verified_modes,
                    unavailable_physical_cases=failed_modes,
                    verified_pairs_only='Successful triplets verify compatible intersections, physical costs added once, unchanged context and occurrence-mean language scores. Failed triplets are explicitly unavailable.'),
        interpretation='Six linked identities, twelve real-scan occurrences. First/second/joint and vocabulary-presence conditions reuse observations. Links and target locations supplied. External joint scoring is our declared adaptation of upstream per-occurrence widths, not their native feature. No unknown-text recovery or end-to-end detection rate follows.')
    a.output.write_text(json.dumps(output,indent=2)+'\n')
    print(json.dumps(conditions,indent=2))


if __name__ == '__main__':
    main()
