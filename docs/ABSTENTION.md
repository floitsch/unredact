# Explicit answer selection and abstention

`assess` adds an opt-in decision to an existing context-ranking report:
`answer` or `insufficient_evidence`. Ranking alone does not make this decision.
There is no built-in accuracy threshold or probability estimate. A policy must
be supplied explicitly, ideally frozen using development documents before its
evaluation on separate documents.

```sh
target/debug/unredact assess \
  --report context-ranking.json --policy policy.json \
  --checks evidence-checks.json --output assessment.json
```

The assessment retains the complete policy, exact input-file SHA-256 hashes,
top-candidate features, external checks, and reasons for abstention. It does not
rewrite the ranking report. An abstention has no `answer_candidate`; its
`features.top_candidate` remains a provisional ranking result.

The initial policy family uses two thresholds: minimum top score and minimum
gap to the second candidate. Scores are taken from an explicit metric, such as
`combined_score`. At least two candidates are required. Ties, missing scores,
nonfinite values, incomplete/truncated reports, missing occurrence context, or
inconsistent physical/contextual membership cause abstention. The policy binds
the report's normalization, physical weight, engine interpretation, and reported
language-model/scorer/adapter/options provenance. Missing or changed language
identity causes abstention, including a model change with identical score units.
Provider requests must use the report's contexts and compatible physical surfaces;
ranked observations must match recorded provider scores. Token, word, sequence,
physical, and combined aggregates are recomputed and checked within `1e-8` log
score units. Policies also record the context-source modes observed in development
(such as paragraph OCR or same-row OCR); unobserved modes are rejected.

For archived `exact_replay_of_common_language_control` providers, the assessor
reads the original cache referenced by the provider arguments and verifies its
SHA-256 against `response.reference_sha256`. It checks contexts, candidate
surfaces, token counts, and log scores before using its model identity. Historical
Rust/Python JSON round trips receive an absolute `1e-9` tolerance for log scores;
the cache-file hash remains exact. These old caches lack scorer-source hashes
and adapter metadata; the signature records those omissions explicitly. They
cannot support a stronger provenance claim. Current direct provider metadata
binds source hashes and adapters when those fields are available.

## Font and link evidence

Score gaps do not detect every missing font or incorrect entity link. The
optional checks file records externally supplied evidence:

```json
{
  "font_coverage": {
    "status": "fail",
    "source": "manual audit: generating font excluded",
    "description": "Known missing-font control"
  },
  "link_validity": {
    "status": "unknown",
    "source": "",
    "description": "Entity correspondence has not been established"
  }
}
```

Allowed statuses are `pass`, `fail`, and `unknown`. A known failure always causes
abstention, even with a large score gap. A pass needs a source. A policy can
require checks using `require_font_check` and `require_link_check`; unknown checks
then cause abstention. Without these requirements, unknown checks remain unknown
and may be permitted. The assessor does not verify user assertions or convert
them into calibrated evidence. Synthetic tests cover known failed font/link
checks, required-but-unknown checks, and unsupported model changes. Real
missing-font and incorrect-link robustness still needs dedicated evaluation.

## Development and independent evaluation

`scripts/evaluate_abstention.py` uses only the Python standard library. Its
manifest lists every attempted case, including pipeline failures. Each successful
case identifies a complete report, its exact SHA-256, document/source group,
present/absent truth status, and whether the unique top-ranked answer is correct.
`top_correct` is an evaluator label tied to that report, never a feature available
to the assessor. `checks` can record independently prepared font/link controls.

`fit` searches thresholds using development data only. Its explicit objective
specifies minimum empirical accuracy, maximum absent-answer wrong acceptance,
and minimum answer count. It maximizes coverage subject to those constraints.
If no rule qualifies, it freezes a disabled policy that always abstains.
Empirical development accuracy is not a guarantee for future documents.

`evaluate` loads the frozen policy and refuses overlap with development document
IDs or source groups. It reports accuracy among answered cases, coverage among
**all attempted cases**, correct-answer yield, absent-answer wrong acceptance,
and failed/unusable reports. Missing OCR/context, missing files, changed report
hashes, and singleton vocabularies remain in the denominator. An additional
curve evaluates threshold pairs frozen on development data before seeing
evaluation outcomes; those evaluation outcomes do not select a replacement rule.
Document IDs/source groups must describe actual shared sources: renaming a file
does not establish independence.

## Archived retrospective experiment

The bounded reproduction is stored in
[`runs/abstention-review-001`](../runs/abstention-review-001). It uses archived
**pre-review-fix** T5 reports, without rerunning OCR, geometry, or model inference.
It evaluates answer selection on those old scores, **not the new scoring or OCR
fixes**. Development uses four documents from `multidoc-common-language-002`;
evaluation uses eight disjoint documents from `heldout-opaque-001`.

```sh
python3 scripts/evaluate_abstention.py prepare-archives \
  --development-summary runs/multidoc-common-language-002/evaluation/summary.json \
  --evaluation-summary runs/heldout-opaque-001/language-evaluation/summary.json \
  --output-dir runs/abstention-review-001

python3 scripts/evaluate_abstention.py fit \
  --manifest runs/abstention-review-001/development-manifest.json \
  --min-accuracy 0.9 --max-absent-acceptance 0.05 --min-answers 5 \
  --output-policy runs/abstention-review-001/policy.json \
  --output-development runs/abstention-review-001/development.json

python3 scripts/evaluate_abstention.py evaluate \
  --manifest runs/abstention-review-001/evaluation-manifest.json \
  --policy runs/abstention-review-001/policy.json \
  --development runs/abstention-review-001/development.json \
  --output runs/abstention-review-001/evaluation.json
```

The selected rule requires combined score at least `-5.724056553717742` and score
gap at least `1.8857297158683686`. These values are specific to this archived
model, vocabulary construction, score combination, and report generation.

| Split / rule | Attempts | Answered | Correct / answered | Coverage | Absent wrong acceptance |
| --- | ---: | ---: | ---: | ---: | ---: |
| Development, selected rule | 192 | 32 | 29/32 (90.6%) | 16.7% | 2/96 |
| Evaluation, frozen rule | 384 | 43 | 43/43 | 11.2% | 0/192 |
| Evaluation, accept every usable unique top | 384 | 275 | 90/275 (32.7%) | 71.6% | 137/192 |

Twelve development reports are unusable: ten lack context and two have no
top-two score gap. Evaluation includes 109 unusable reports: 108 lack context
and one is a singleton. They remain in the attempt totals. The evaluation JSON
contains 181 development-frozen accuracy/coverage curve points and per-case
decisions; it identifies zero-answer rules with `selective_accuracy: null`.

The frozen rule answers a small fraction of cases. Its observed 43/43 does not
establish 100% future accuracy; cases are clustered within only eight documents,
and present/absent controls reuse masks. These archives do not measure missing
fonts or incorrect links. Font/link checks are unknown and permitted by this
particular retrospective policy. New-engine reports have a different signature
and need newly generated development/evaluation data and a new frozen policy.
