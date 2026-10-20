# Matched comparison results

The selected Rust geometry plus local T5 pipeline outperforms the pinned external
baseline controls on the independent multidocument test. This is a measured
advantage on the declared benchmark, with supplied candidate vocabularies and mask
locations. It does not establish recovery accuracy for unknown real redactions.

These are frozen historical results. Subsequent dependency-grouping,
compatibility and OCR changes have regression tests and bounded exploratory
checks, but the complete benchmark below has not been rerun with those changes.
The optional [abstention evaluation](ABSTENTION.md) also uses archived reports;
its policy is bound to that report generation and language-model provenance.

## Opaque masks

All 192 held-out spans from eight documents are included. Each vocabulary contains
300 candidates, including the correct text. Every language-based method receives
the same cached T5 sequence scores. Model and settings were selected using four
different development documents before test scoring.

| Method | Correct first / 192 |
| --- | ---: |
| Selected Rust filter + T5, physical weight 1 | **90** |
| Rust filter + T5, weight 0 control | 86 |
| T5 alone | 69 |
| Alex matched-catalog/wide filter + identical T5 | 46 |
| Unredactron native selection, neutral CSV confidence | 9 |
| Unredactron CSV hook + identical T5, scale 1 | 9 |
| Unredactron CSV hook + identical T5, scale 10 | 12 |
| Unredactron CSV hook + identical T5, scale 100 | 47 |

Rust/Alex values require a unique first place. Unredactron values credit its native
selected match, including its own tie choice. Scales 1 and 10 reflect documented
or sample CSV conventions; scale 100 is a declared stress control. All scales are
reported, without selecting one from test results. These supplementary native
controls were registered after geometry completed and before full language
results. Alex's optional paid Claude integration was not run. Unredactron has no
built-in automatic LLM reranker or vocabulary-wide compatibility API.

Paired document-cluster bootstrap differences for the selected method:

| Comparison | Difference in percentage points | 95% interval |
| --- | ---: | ---: |
| Alex + common T5 | +22.92 | +11.98 to +33.33 |
| Unredactron T5 scale 100 | +22.40 | +8.85 to +36.46 |
| T5 alone | +10.94 | +4.69 to +17.19 |

The registered sensitivity gives each of five publisher families equal weight.
The intervals remain positive: +25.56 points versus Alex [11.67, 36.39] and
+28.33 versus Unredactron scale 100 [10.00, 45.83]. These are descriptive intervals
for a small source sample, not guarantees across all document producers.

There are **54 missing-context failures per availability control**, including all
48 spans from the two Congressional Record scans. They remain incorrect in the
192-case denominator. Of the other failures, two are in NIST, two in USGS water
use, one in NASA and one in USGS impact. Alex has one additional geometry failure.
On real scans alone, selected Rust/T5 scores 32/96 versus Alex/T5's 12/96; on clean
sources the counts are 58/96 and 34/96.

Width-only ordering does not outperform the baselines: Rust uniquely ranks 2/192
first versus Alex's 5 and Unredactron's 9. Rust retains 188 correct candidates versus
Alex's 83, but its median compatible set is 66.5. Language ordering resolves some
of that ambiguity; retention alone is not recovery.

The 192 absent-answer controls are diagnostic rather than a calibrated rejection
test. Rust rejects no entire vocabulary. Unredactron accepts 190 wrong selections
under each confidence scale. A first-ranked candidate cannot establish that the
original text was supplied.

## Pixelation

The same 192 spans are evaluated with two pixelation sizes and paired present/
absent vocabularies: 768 dependent cases, 2,304 engine executions. This comparison
is against this repository's previous scorer, not external pixelation tools.

| Correct unique first | Previous single-crop scorer | Shared cells, unit height | Selected shared cells, learned height |
| --- | ---: | ---: | ---: |
| 8-pixel blocks | 43/192 | 115/192 | **116/192** |
| 16-pixel blocks | 0/192 | 88/192 | **87/192** |

Both shared-cell modes complete every case. The previous scorer has 414 failures
across the 768 cases, retained in the denominator. Shared-grid detection, partial
cell scoring and position search improve over that scorer. Learned height does
not show a consistent independent gain over unit height. The original selected
method remains fixed. No calibrated absent-answer accuracy is claimed.

## Implementation and verification

The app includes HarfBuzz/FreeType kerning measurements, visible-text calibration,
native PDF coordinates, relevant-name/alias inputs, local model proposals and
ranking, shared-grid pixelation scoring, and explicitly linked evidence across
PDFs with duplicate/dependence handling. The requested 93,016-parameter font CNN
is trained and integrated, but remains optional: a real-scan ranking advantage
over ordinary font search has not been established.

The archived evaluation binary passed 122 Rust tests, all-target Clippy and the release build.
The completion audits check all 552 successful language CLI rankings and all
1,890 successful pixelation reports against their scores, coverage and ranks.
All 384 neutral native replays reproduce the saved upstream decisions before
confidence adaptation. Frozen inputs, source, models and runtime hashes verify.
The trained CNN also runs through the current Rust CLI using 24 independently
audited montages from 60 visible training words, excluding masks and held-out text.

The corpus has 288 spans from 36 pages of 12 PDFs, six clean and six real scans.
Six damage conditions generate 3,456 inputs; independent solver results above
cover opaque masks and the two pixelation conditions. The other damage fixtures
are available for further evaluation. Variants are not independent samples.

Remaining limits include public English government text, readable printed-text
selection, possible model pretraining exposure, manually supplied masks and links,
closed candidate vocabularies, and substantial OCR/context failures. Open-vocabulary
proposal recall and actual Epstein-file identity recovery are not measured by
these results. Arbitrary mask padding, handwriting, unknown fonts and strong
post-pixelation resampling need further validation.

## Reproduction and evidence

Use the [quickstart](QUICKSTART.md) to run the selected pipeline. The complete
[benchmark protocol](BENCHMARK_EXPANSION.md) records selection, damage, source pins,
adaptations and chronological development results. The [source review](BASELINE_SOURCE_REVIEW.md)
explains how the two external implementations differ.

Authoritative artifacts:

- `runs/heldout-ranking-audit-001/final-analysis.json`: full common-language results.
- `runs/heldout-opaque-001/geometry-summary.json`: full geometry comparison.
- `runs/heldout-native-decisions-001/confidence-controls.json`: every native scale.
- `runs/heldout-native-analysis-001/paired-analysis.json`: supplementary paired analysis.
- `runs/heldout-analysis-001/family-sensitivity.json`: registered family sensitivity.
- `runs/heldout-pixelation-001/evaluation/summary.json`: full pixelation results.
- `runs/heldout-pixelation-001/completion-audit.json`: actual pixelation output audit.
- `runs/completion-audit-001/requirements.json`: requested-feature completion evidence.
