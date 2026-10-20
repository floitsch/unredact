# Multi-document controlled-redaction benchmark

The independent runs and completion audits have finished. See
[completed results](RESULTS.md) for the current comparison and limitations. The
sections below preserve the protocol and development chronology.

The earlier eight spans from one NASA report are development checks. They are
not sufficient evidence of general accuracy, and further tuning on them does
not produce an independent test result.

`benchmarks/multidoc-v1/plan.json` fixes source documents, document splits,
sampling seed, page budget, label rules and damage conditions **before scoring**.
The first generated set contains 12 public government PDFs, half clean PDFs and
half real scans, with three pages and eight spans per document: 288 original spans.
Four documents are development; eight are test. The manifest records actual
counts and any unmet quotas. A source download replacement is recorded in the
plan; sources are never replaced because a solver performs poorly.

This is an initial broader sample, not a claim that twelve documents represent
all redacted material. The sources are English government publications. Agency,
report series and typography can still be correlated across different documents.
Handwriting, names, arbitrary masks, automatic localization, cross-document link
discovery and open-vocabulary proposals need additional evaluation tracks.

## Generate cases

```sh
python3 scripts/fetch_benchmark_sources.py \
  --plan benchmarks/multidoc-v1/plan.json \
  --output runs/multidoc-sources-001

runs/baseline-venv/bin/python scripts/prepare_multidoc_benchmark.py \
  --plan benchmarks/multidoc-v1/plan.json \
  --sources runs/multidoc-sources-001/sources.json \
  --output runs/multidoc-benchmark-001
```

The source manifest supplies local PDF paths and SHA-256 hashes. The generator
uses Poppler, Tesseract, Pillow and NumPy. It records hashes and renderer/OCR
versions. Pages are sampled in deterministic hash order, with a fixed inspection
budget. Non-text pages, OCR disagreement, low-confidence words, neighbor overlap,
page errors and quota shortfalls are recorded. The resulting sample is explicitly
conditional on readable original printed text; degradation is applied only after
selection. Four single words and four adjacent two-word spans are selected on
each accepted page. No substitutions are chosen using solver outcomes.

Clean-PDF labels must agree with native text at the same location. Scan labels
are original-image OCR proposals. Contact sheets must be visually checked before
calling these ground truth. The summarizer refuses unverified labels by default;
`--allow-provisional-labels` is only for debugging the evaluation pipeline. Review concerns label/rectangle correctness, never
which method wins. PDF originals, OCR and labels stay in `controller/`; adapters
receive flattened PGM images, mask locations and candidate lists. A black overlay
PDF with recoverable hidden text is never supplied. This directory separation is
a protocol boundary, not an operating-system security sandbox.

Each source span has six paired image conditions:

| Condition | Operation |
| --- | --- |
| Opaque | Full black mask at 240 DPI, unknown total horizontal padding 0–12 pixels |
| Reduced resolution | Mask, downsample to half resolution, then resample to common coordinates |
| Scan damage | Mask, Gaussian blur 0.7 px, noise SD 3, JPEG quality 40 |
| Subpixel shift | Mask, then fractional translation (0.375, 0.25) px with bicubic interpolation |
| Pixelation 8 | Page-aligned 8×8 block averages inside the target regions |
| Pixelation 16 | Page-aligned 16×16 block averages inside the target regions |

All selected spans are obscured on every supplied page. Opaque damage happens
**after** masking, so blur and compression cannot expose text that was supposed
to be removed. An invariance test changes the underlying hidden pixels and
requires identical output for every opaque condition. Pixelation deliberately
retains information and is reported separately. This covers one order of
operations; pre-redaction blur and photographic perspective are further tracks.

The pixelation input uses a block search of 4–20 and an origin/baseline estimate
from the mask alone. Those estimates are imperfect on purpose: native glyph
origins, true fonts, true block size and hidden-text measurements are not supplied.
It is not an oracle-aligned renderer test. External tools may return unsupported;
that must be reported, not treated as a valid comparison win.

Each condition has paired correct-candidate-present (300 candidates) and
correct-candidate-absent (299 candidates) controls. The distractor pool mixes
ordinary words and artificial word pairs from an existing dictionary. Injecting
the answer into a closed list tests filtering/ranking, not discovery. This is not
a realistic language prior or a substitute for measuring LLM proposal recall.
The expected maximum is 3,456 case/control inputs, **not** 3,456 independent tests.

## Run and report

Suite files are separated by development/test, damage condition and availability.
Use the existing pinned adapter manifest with the ordinary Rust `compare` command.
Keep test suites untouched while choosing models, thresholds and scoring weights
on development sources. Once test outcomes guide a change, label that test cohort
as development and create a new document-level holdout.

```sh
target/release/unredact compare \
  --suite runs/multidoc-benchmark-001/development-opaque-present.json \
  --adapters runs/combined-ranking-holdout-001/adapters.json \
  --output runs/multidoc-development-opaque --timeout-seconds 240

runs/baseline-venv/bin/python scripts/summarize_multidoc_benchmark.py \
  --benchmark runs/multidoc-benchmark-001 \
  --comparison runs/multidoc-development-opaque/comparison.json \
  --output runs/multidoc-development-summary.json
```

The summarizer verifies suite hashes and rejects duplicate executions. It reports
source-group macro averages and paired differences with 10,000 document-cluster
bootstrap draws. Multiple spans, degradation variants and repeated executions
are not treated as independent samples. Intervals are descriptive for the chosen
source sample and may remain wide or unstable with few documents per stratum.
Failures/unsupported executions count as incorrect and are tallied separately;
missing executions make a stratum incomplete and suppress its accuracy/interval.
Absent-case empty-compatible-set results are only an abstention proxy; a method
that does not expose compatibility cannot be credited with rejecting an answer.

Public pretrained models may already have encountered these documents. Document
splits prevent this project's fitting/evaluation leakage, not pretraining exposure.
Exact file/pixel duplicates are rejected; near-duplicate editions still require
source review and a common `group` before freezing the experiment.

## First generated cohort

`runs/multidoc-benchmark-001/audit-final.json` verifies all 288 spans, 3,456 cases,
216 damaged page images, source-group split integrity, complete paired coverage,
flattened raster inputs and candidate availability. The source manifest is
`runs/multidoc-sources-001/sources.json`. All 12 documents met their quotas; one
sampled USGS page lacked sufficient eligible text and is recorded as an exclusion.

There are 96 development spans and 192 test spans; each split has equal numbers
from clean and scanned sources. There are 144 one-word and 144 two-word spans.
All 144 scan transcriptions were visually checked against the 18 saved contact
sheets; all 144 clean-source labels passed native-text/location agreement.
No labels were corrected or removed. Printed line-start fragments on the 1970
Congressional Record page remain exact fragments, without guessed completion.
The label review and all source URLs/hashes are saved with the cohort.

Sources come from NIST, Census, USGS, NASA and two Congressional Record years. It remains concentrated
in English US government publishing; differences between editions from one
institution do not make them fully independent domains. Neither these counts nor
a pipeline smoke test establish a new accuracy or superiority result.

## Natural distractor control

The original dictionary contains many rare nouns and artificial word pairs. It
can probe geometric ambiguity, but it is weak evidence for language-model gains.
`scripts/prepare_natural_candidates.py` creates a separate control without changing
any images, targets, labels, splits or damage conditions:

```sh
runs/baseline-venv/bin/python scripts/prepare_natural_candidates.py \
  --benchmark runs/multidoc-benchmark-001 --output runs/multidoc-natural-001
runs/baseline-venv/bin/python scripts/audit_natural_candidates.py \
  --benchmark runs/multidoc-natural-001 \
  --output runs/multidoc-natural-001/candidate-audit.json
```

The generated bank has 1,180 distinct single tokens and 1,246 two-token phrases
from 4,232 visible occurrences on the twelve development pages. Mask intersections,
low-confidence words and invalid tokens prevent harvesting and block phrases.
No test-page OCR is read. Original review images remain in the parent benchmark,
identified by `vocabulary-provenance.json`; original image bytes are shared through
read-only references and checked by hash.

Each closed list has exactly150 single tokens and150 two-token phrases. Sampling
uses neither the hidden answer's width nor semantic similarity. If absent from
the random draw, the answer replaces one entry in the appropriate class, keeping
class counts fixed. The matching absent list removes it. This remains an explicitly
supplied-candidate task. Non-injected draws and their pool/sample availability are
saved separately as construction diagnostics, not a blind model-proposal test.
Those diagnostics must not be used to tune a purported held-out retrieval result.

All3,456 modified inputs pass the structural and provenance audits. The development
opaque subset has since been used in the padding ablation below; other conditions
and test outcomes remain unscored. Two tests verify mask/gap barriers and that answer injection
cannot change the one-/two-token balance or the independent non-injected draw.

## First broad geometry run completed

`runs/multidoc-development-001/plan.json` fixes the first full development
comparison: all96 opaque present cases and their96 absent controls, three pinned
configurations, one repetition,240-second per-execution timeout, and
`OMP_THREAD_LIMIT=1`. The present and absent suites run concurrently. Existing
Rust, Alex matched-catalog/wide-tolerance, and Unredactron matched-catalog adapters
are unchanged. This is576 executions over four source documents. Other damage
conditions and test-document results are not included in this run.

`scripts/analyze_multidoc_geometry.py` reports exact-first success, best-group
membership, conservative/optimistic top-five ties, retention, abstention proxies,
failures and document-cluster paired intervals. It leaves unexposed compatibility
metrics null, and reports a separate both-executions-successful comparison without
silently dropping failures from the primary denominator. It rejects incomplete
suites, duplicate trials, unmatched adapter inputs and invalid provenance. Do not
interpret partial progress counts as accuracy results.


All576 executions in `runs/multidoc-development-001` completed successfully, with
matched inputs, valid source provenance and frozen files verified at completion.
Results on the96 supplied-answer development spans:

| Configuration | Unique correct first | Truth compatible | Median compatible candidates |
| --- | ---: | ---: | ---: |
| Default Rust profile cost | 2/96 | 95/96 | 40.5 |
| Alex matched catalog, wide tolerance | 9/96 | 41/96 | 9 |
| Unredactron matched catalog | 6/96 | Not exposed | Not exposed |

Missing-answer controls produced zero empty compatible sets for Rust and six for
Alex, out of96 each. Unredactron does not expose a compatible set. Thus high
retention comes with substantial ambiguity; it is not reliable absence detection.
Paired, document-cluster statistics and per-document results are in
`geometry-summary.json`. These are four development documents, not a general
accuracy estimate. The existing tools beat the default Rust geometry scorer on
unique first rank in this cohort.

An opt-in Rust `opaque_uniform` prior was subsequently tested with identical
cached OCR/profiles on all96 spans and their absent controls. Its unique-first
count rose from2 to13 on dictionary candidates and from1 to6 on natural candidates;
retention stayed95/96. But expected credit for random selection within the old
best groups was10.01 and7.61 respectively. The natural-bank result therefore
fails to establish a consistent ranking gain. Both fractional-credit difference
intervals include zero. The prior remains explicitly selected, not the default.
See `runs/padding-uniform-development-001/evaluation/tie-aware-final.json`.

The new Rust source passes105 tests and Clippy. A fresh adapter integration check
on one development input per source exactly matches the corresponding cached
prior reports; it is only a pipeline check, not another independent accuracy
sample. Historical files changed during integration were archived under
`runs/multidoc-development-001/before-padding-integration`; their mapping is
`integration-snapshots.json`. Verification of the completed original run now uses
those archived bytes for changed files, not the newer implementation.

## Blinded model inputs and equal-size controls

Before any language/pixelation comparison, `scripts/blind_multidoc_inputs.py`
removes fixture construction cues from requests. Earlier target IDs encoded span
word count, image filenames encoded pixelation block size, and absent lists were
one candidate shorter. The geometry scorers used in the completed run do not
consume those cues as prediction features, but future model inputs must not
expose them.

`runs/multidoc-blinded-001` contains all3456 corresponding inputs with SHA-256
aliases, shuffled case order, and content-addressed regular image files. Image
hard links preserve exact bytes without revealing original names through path
resolution. Both present and absent lists have300 candidates,150 single tokens
and150 two-token phrases. Absent controls add an unused same-class visible-source
distractor. Private controller mappings preserve every original identity, label,
location and image hash. An exact reverse-mapping audit verifies all changes;
source pixels/masks/evidence are unchanged. This is protocol blinding, not an OS
sandbox. Evaluation output directory names should also avoid revealing conditions.

The next full geometry comparison uses this blinded natural bank, all96 development
spans and96 absent controls, default and uniform-prior Rust plus both external
baselines. Its frozen plan is `runs/multidoc-language-development-001/plan.json`.
The two opaque output-directory names map to availability only in the controller
plan. Geometry results will be kept separate from subsequent common-language
ablations; test-document model outcomes remain untouched.

## Full common-language comparison

`scripts/evaluate_multidoc_language.py` evaluates every development opaque span
and its missing-answer control: 96 spans, 192 cases, 300 candidates per case.
The fixed plan is `runs/multidoc-common-language-001/plan.json`. Its launch waits
for both completed geometry reports and verifies full coverage, provenance, and
frozen files before scoring. Partial geometry results are not graded.

The pinned local T5 model scores all candidates using the same visible context
for language alone, each Rust compatible set, and Alex's compatible set. Rust
rankings with physical weight zero and fixed weight one are replayed through the
CLI using exact cached scores. Default profile cost and the optional uniform
padding prior remain separate. Unredactron's compatible set is unavailable; no
cutoff is inferred from its ranks. These are common-language controls, not tests
of upstream Claude integration.

Reports retain score ties, planned failure denominators, document-level paired
intervals, source-kind and word-count breakdowns, and missing-answer diagnostics.
An empty compatible set is only an operational abstention proxy. Four development
documents cannot support a broad performance claim, and pretrained-model exposure
to these public documents is unknown. The eight test documents remain unused for
model selection. Three evaluation tests pass; a single 300-candidate pipeline
check verifies all four Rust replays, without treating it as an accuracy sample.

## Grid detection on short real-document crops

`scripts/audit_multidoc_grids.py` runs the existing Rust detector on every one of
the 96 development spans under seven paired conditions: two pixelation sizes,
four opaque-mask variants, and the original visible-text crop. All672 executions
in `runs/multidoc-grid-audit-002` succeed. The detector retains the exact grid on
75/96 eight-pixel crops and0/96 sixteen-pixel crops. It detects no grid in the480
negative controls. All96 sixteen-pixel crops are too short for the detector's
requirement of three boundaries in both axes. The first audit attempt stopped
before scoring because fractional-shift crop bounds needed floor/ceil rounding;
the completed audit matches Rust's crop convention.

An isolated Rust prototype pools derivative evidence across disjoint regions in
page coordinates, then checks within-cell constancy and individual region support.
It rejects overlapping regions instead of counting duplicate pixels. No hidden
text, block size, phase, or glyph geometry is supplied. On the same development
pages it retains the exact grid in96/96 crops for each block size, with zero
detections in all480 negative controls. Reports and source are preserved in
`runs/shared-grid-development-001`; the isolated full suite passes107 tests and
Clippy. This uses additional evidence from eight declared regions per page and
assumes a common page grid. The192 positive crops represent24 paired page-level
conditions across12 pages/four documents, not192 independent documents. This is
grid detection only; the prototype is not yet integrated into candidate scoring.
No test-document results were used.

The audit also exposed a separate coordinate error in the benchmark generator:
its mosaic text origin/baseline were page-relative, while Rust renders inside the
target crop. Before any multidocument pixelation text-ranking run,
`scripts/correct_mosaic_coordinates.py` produces
`runs/multidoc-blinded-coordinates-001`. Exactly1152 pixelation cases are amended
among3456 inputs; subtracting the rounded crop origin uses only mask geometry.
Pixels, candidate sets, labels, splits, and all opaque suites remain unchanged.
This preserves the original approximate positioning rather than supplying hidden
ink origins or baselines. A regression test covers fractional coordinates and
rejects accidental double application. Use this amended bundle for subsequent
pixelation text evaluations. The ongoing opaque geometry/language runs and grid
audits are unaffected by this fixture correction.

## Blinded natural-candidate geometry results

Both frozen suites in `runs/multidoc-language-development-001` completed with
all768 executions successful. The96 supplied-answer development cases produce:

| Configuration | Unique correct first | Truth compatible |
| --- | ---: | ---: |
| Default Rust profile cost | 1/96 | 95/96 |
| Optional uniform-padding cost | 6/96 | 95/96 |
| Alex matched catalog, wide tolerance | 8/96 | 41/96 |
| Unredactron matched catalog | 7/96 | Not exposed |

On96 absent-answer controls, both Rust modes produce zero empty compatible sets,
and Alex produces eight. Unredactron does not expose that decision. Source/runtime
hashes and complete execution coverage pass the completion audit. The common-
language evaluation is running from these completed results. High candidate
retention alone has not established superior ranking or absence detection.

## Partial-cell pixelation scorer experiment

`runs/mosaic-cells-development-001` freezes an isolated Rust experiment that uses
the shared grid to rank actual candidate strings. Its forward model assumes
page-grid averages copied inside the masks, while outside pixels remain unchanged.
For each intersected cell it combines rendered hidden pixels with the observed
outside contribution. Cells touching another mask are excluded because that
contribution is unknown. This supports crops containing no complete cells.

Candidates are rendered with the cached visible-text font fits, cropped to their
complete ink bounds, and searched over all integer positions inside the mask at
four rendering phases. Integral darkness sums avoid rerendering for each integer
translation. Clipped strings are rejected; unsupported candidates remain in the
output with null cost. The fixed sigma8 and font-validation penalty are experimental
costs, not calibrated probabilities. No true hidden origin or baseline is supplied.

The full planned evaluation has384 cases:96 development spans, two pixelation
sizes, and paired present/absent300-candidate controls. The current scorer receives
the coordinate-corrected inputs; both engines use identical cached typography.
Every pixel outside the supplied masks must match the opaque calibration image
before cache reuse. This isolates scorer changes and does not measure fresh OCR.
Failures remain in denominators; no absence accuracy is claimed without a rejection
rule. The experiment runs separately from the frozen main application.

The isolated Rust suite passes110 tests and Clippy. Tests cover independently
constructed partial-cell averages, nonzero phases, clipped page-edge cells,
unknown contributions from other masks, translations, and complete-string bounds.
Two evaluator tests cover visible-pixel cache invariance and failure denominators.
An ungraded300-candidate integration request completes successfully.

The full384-case scorer comparison subsequently completed. All384 shared-cell
executions succeed; the previous scorer succeeds on186 and rejects198 because
the single-region grid detector cannot infer a supported grid. Planned failures
remain in the ranking denominator. On the supplied-answer controls:

| Pixelation | Previous scorer, correct first | Shared-cell scorer, correct first | Shared-cell truth renderable | Shared-cell top5 guaranteed |
| --- | ---: | ---: | ---: | ---: |
| 8-pixel blocks | 24/96 | 54/96 | 68/96 | 64/96 |
| 16-pixel blocks | 1/96 | 35/96 | 68/96 | 53/96 |

The document-cluster intervals for the paired correct-first improvements are
[0.104,0.583] and[0.125,0.583], respectively, across only four development
documents. These are the same96 spans in two conditions. The comparison measures
the combined grid/position/partial-cell changes against this repository's previous
scorer, not against external projects. It does not establish general superiority.
The prototype remains isolated while the main application's frozen language
evaluation runs.

`scripts/audit_mosaic_forward_model.py` checks the completed comparison using the
original pixels exclusively in a private controller audit. All192 positive cases
reproduce cell values within8-bit rounding;18 sixteen-pixel crops have no complete
cells. Thus this forward process matches the generated fixtures. The true text
cannot be rendered inside the mask by any supplied font fit on28/96 spans, and
rendered glyph residuals are much larger than the oracle's rounding residual.
This identifies typography/rendering as a remaining limitation. Original hidden
pixels never enter candidate scoring. Missing-answer ranking remains uncalibrated;
no absent-answer accuracy or confident identification is claimed.

## Rendering-height diagnosis and calibration

`runs/render-dimensions-development-001` measures all candidates and font fits
on every development span, then uses private labels to inspect the true candidate.
It exactly reproduces the previous scorer's68 renderable truths. Of the28 failures,
26 have all font fits too tall and two have all fits too wide. This identifies a
specific limitation of using width-fitted font sizes for image synthesis.

An isolated rendering experiment in `runs/vertical-mosaic-development-001` learns
a separate vertical scale from visible training-word boxes. For each retained
font fit, the scale is the median OCR-height/outline-height ratio, requiring12
training words and accepting scales0.25–4. Other cases retain unit scale. Visible
training, validation and audit text sets must be disjoint. Neither target dimensions
nor hidden labels select the scale. A Cairo transform applies it to rendering while
preserving horizontal size/tracking. Unit scale uses the original renderer exactly.

On the first width-selected fit from each of12 unique page profiles, visible
validation height RMSE falls6.284→1.890 pixels across603 word observations; audit
RMSE falls6.659→1.994 across285. Font alternatives and repeated target spans are
not counted as additional words in those totals. The full384-case masked-span
comparison is running and must reproduce every original candidate score at unit
scale before measuring learned-scale ranking. The isolated113-test Rust suite and
Clippy pass, including tests that held-out heights cannot alter the fitted scale.

The original common-language run stopped after45 cases when a target lacked usable
visible context. `scripts/evaluate_multidoc_language_v2.py` resumes it into
`runs/multidoc-common-language-002/evaluation`: exact cached scores are reused only
after checking model, context, candidate text, tokens and sequence sums. Missing
context becomes a planned case failure instead of stopping the suite. No scoring
settings or source documents changed. The original partial run is retained.
`scripts/finalize_multidoc_language.py` will keep an unexposed upstream compatibility
API unavailable even when that same case also has a context failure; such a missing
API cannot acquire a measured zero accuracy from the context error.

The resumed run completed all192 planned cases. Forty-five score sets were reused
after exact checks;182 cases scored and ten cases (five spans in both availability
controls) lack usable context. Those failures stay in the denominator. Final
common-language supplied-answer results are43/96 language-only,56/96 default
geometry filter,58/96 default filter plus fixed physical weight1, and33/96 Alex's
filter. The optional uniform-prior weight1 gets59/96; its one-case advantage does
not replace the selected default. Unredactron remains unavailable for this common-
language filter comparison. All scoring uses the identical local T5 model and
candidate scores, not upstream Claude. Final audited results are in
`runs/multidoc-common-language-002/final-analysis.json`.

The vertical-scale experiment also completed: all768 executions succeed, and all
384 unit-scale score arrays/grid diagnostics/cell models exactly reproduce the
prior scorer. Learned height scaling improves correct-first ranking54→69/96 for
8-pixel blocks and35→53/96 for16-pixel blocks. Truth renderability rises68→85/96.
These gains use training-only visible words; the same96 development spans appear
in both conditions. Paired document-cluster improvement intervals are[.083,.271]
and[.135,.260] across only four documents. No absence calibration is claimed.

Validated shared-grid, partial-cell and vertical-rendering modules are now in the
main Rust library, with `analyze-pixel-regions` and `rank-pixelation` CLI commands.
The latter accepts existing projects, profiles, candidate files and mask locations;
`--learn-vertical` excludes every supplied mask from height learning. Main tests and
Clippy pass. Twenty-four CLI replays (two modes on12 existing pages) exactly match
the frozen experiment's candidate scores/grid analyses/cell models; these are
integration checks. Original main files/binary were archived before integration in
`runs/main-before-mosaic-integration-001`; `scripts/verify_archived_freeze.py` verifies
historical hashes against those bytes without rewriting old experiment manifests.

## Independent held-out comparison launched

`runs/heldout-opaque-001/plan.json` fixes the selected default geometry plus local
T5 physical weight1 before any test scoring. All192 spans from eight held-out
documents and their192 absent controls are included. Controls retain300 candidates;
the distractor vocabulary comes from visible development text. Language-only and
weight0 are controls. Both pinned external geometry adapters run on matched inputs;
Unredactron's missing compatibility API stays unavailable for language filtering.

The binary, adapter/evaluator source and main source are copied into an immutable
runtime snapshot. Main application integration cannot change the held-out method.
Both full geometry suites have completed; the complete384-case language comparison
is running. No partial language accuracy has been inspected, and no test
outcome may select fonts, hyperparameters or model variants. Complete language
results remain pending; overall outperformance is not established.

## Independent pixelation test and linked-occurrence integration

`runs/heldout-pixelation-001/plan.json` prospectively fixes all192 independent test
spans across eight documents, both pixelation sizes and both availability controls:
768 cases, three methods each. The selected method is partial-cell/shared-grid
scoring with visible-training vertical scale; unit scale and the old single-crop
scorer are controls. Runtime binary/source/scripts are immutable copies. The run
waits for complete held-out opaque typography caches and requires identical
outside-mask pixels before reuse. Failures stay in denominators. No partial test
accuracy has been inspected or used to choose settings.

The main app now exports per-font/per-grid candidate costs in pixelation reports
and provides `combine-pixelation` for explicit links across pages/PDFs. It minimizes
the joint cost with one grid per page and one font per page/profile, charging the
profile penalty once. Reports need identical canonical candidate IDs, but alias
surfaces may differ. Exact copies and supplied dependence keys count once;
same-page reports with inconsistent mask sets are rejected. These are conditional
identity-link hypotheses, not automatically verified identities.

Seven joint-scoring tests cover cases where independent per-occurrence minimization
would incorrectly switch fonts or grids, single-counted penalties, duplicates,
rescaled-copy dependence keys, aliases, missing hypotheses, ties and inconsistent
metadata. Two further tests cover image-shape identity and font-path aliases.
All122 Rust tests and Clippy pass. Twenty-four existing-page CLI replays
show the original solo scores/grid diagnostics/cell models are unchanged and
duplicate exports do not strengthen evidence. Two cross-PDF integration groups
(seven reused development occurrences) verify all300 candidate costs against the
independent-page sums and exclude added duplicate exports. These are integration
checks, not a small-sample recovery-accuracy benchmark.

Current main source/binary before this integration is archived in
`runs/main-before-joint-pixelation-001`. Both held-out runtime snapshots still pass
their frozen-file checks after main changes.

`runs/heldout-analysis-001/plan.json` registers a secondary publisher-family
sensitivity analysis before either complete test report exists. The eight test
documents come from five publisher families (NIST, Census, NASA, USGS and the
Congressional Record). Equal-family means and fixed-seed family-cluster intervals
will accompany the original document/case metrics. This keeps shared publishing
templates visible in the uncertainty assessment; it does not select another
model, alter the primary metric or use partial test accuracy.

## Completed held-out geometry and supplementary native controls

Both opaque geometry suites completed all 1,152 planned executions. Local and
Unredactron succeed on all 384 cases each. Alex succeeds on 382; one source span
in both availability controls fails because its OCR finds no overlapping line.
That failure remains in the denominator.

| Correct-answer-present, 192 spans | Correct retained | Unique correct first | Median compatible set |
| --- | ---: | ---: | ---: |
| Local default geometry | 188 | 2 | 66.5 |
| Alex matched catalog/wide tolerance | 83 | 5 | 13 |
| Unredactron matched catalog | Unavailable | 9 | Unavailable |

The local filter retains more correct candidates but is highly ambiguous. Width-
only ordering has not beaten either baseline. In absent-answer controls, the local
filter produces no empty sets, while Alex produces six; these are set diagnostics,
not calibrated absence probabilities. The complete common-language test is still
running and no scoring setting changes based on these geometry results.

`runs/heldout-native-decisions-001` adds supplementary controls registered **after
geometry completed and before the complete language report exists**. The upstream
Unredactron loop subtracts CSV confidence/10 from percent width error, selects one
candidate, and emits it only when its raw width error is strictly below 30 percent.
This is a selected-match gate, not a compatible-set API. Saved actual zero-confidence
outputs accept 190/192 cases in each availability control. Only 9 of the present-
answer selections are correct; all 190 accepted absent-answer selections are wrong.

The added control feeds the identical cached T5 relative sequence weights through
that existing CSV-confidence field at fixed scales 1, 10 and 100. The first two reflect
documented/sample input conventions; 100 is an explicitly declared percentage-style
stress control. All scales are reported without selecting a winner on test data.
No extra language inference or invented geometric cutoff is introduced. A cached-
width replay executes the actual pinned source loop and must first reproduce the
native zero-confidence output exactly. Development validation reproduces all 192
decisions. Missing language context falls back to native zero confidence. Tests
also check strict boundary rejection, stable ties, prior ordering, and the case
where a prior selects a bad-width candidate that the native raw-error gate rejects.

`runs/heldout-native-analysis-001` registers paired document and publisher-family
intervals for every fixed confidence scale against the selected primary. This
supplementary analysis was fixed after geometry/pixelation completed and before
complete language outcomes. It retains failures and credits native selected
matches, including the upstream tie choice. It selects no confidence scale and
leaves the original primary comparison unchanged.

`runs/heldout-ranking-audit-001` will independently check the actual CLI sorted
candidate lists and tie groups, in addition to the evaluator's per-score checks.
It also preserves unavailable compatibility interfaces as unknown on operational
failures. All 364 successful default-mode development CLI reports pass the same
ordering/group/grade audit. Original scores and raw summaries remain unchanged.

The main fixture generator now writes crop-relative mosaic origin/baseline values
directly. Its previous source is preserved in
`runs/benchmark-generator-coordinate-fix-001/before`. A full 3,456-case audit matches
the corrected frozen corpus's evidence fields using mask bounds alone. Existing
fixtures and all held-out runtime snapshots remain unchanged; future generation
does not require the historical coordinate-amendment step.

## Completed independent pixelation comparison

All 768 planned cases and 2,304 engine executions completed across the same 192
held-out spans, two block sizes and both candidate-availability controls. Both
shared-cell modes succeed on all 768 cases each. The previous single-crop scorer
succeeds on 354 and fails on 414; failures stay in the denominator. Original input,
runtime and source hashes pass the evaluator's end-of-run checks.

| Supplied-answer cases | Previous single-crop scorer | Shared cells, unit height | Selected shared cells, learned height |
| --- | ---: | ---: | ---: |
| 8-pixel blocks | 43/192 | 115/192 | 116/192 |
| 16-pixel blocks | 0/192 | 88/192 | 87/192 |

Values are unique correct first ranks. The selected method's paired document-
cluster improvement over the previous scorer is 0.3802 with 95% interval
[0.2344,0.5156] for 8-pixel blocks, and 0.4531 [0.2448,0.6510] for 16-pixel blocks.
This comparison is against this repository's previous scorer, not external projects.
For real-scan sources alone, the selected method ranks 49/96 and 31/96 correctly
first, respectively; clean-source counts are 67/96 and 56/96.

The learned-height adjustment adds one correct result at 8 pixels and loses one at
16 pixels relative to unit height. Paired document intervals for those differences
are [-0.0365,0.0521] and [-0.0729,0.0521]. Its strong development gain does not show
a consistent held-out ranking advantage. The prospectively selected method stays
unchanged; these test outcomes do not choose another configuration. Renderable
truths increase only 172→173/192. The registered equal-publisher-family sensitivity
still waits for the complete language report.

All rankings are conditional on the supplied 300-candidate vocabularies and known
mask locations. The opaque-reference visible typography is reused only after
outside-mask pixel equality. No calibrated absent-answer decision or fresh-OCR
accuracy is measured. Source artifacts are under
`runs/heldout-pixelation-001/evaluation`; its `unit_control` summary uses the field
name `legacy` for the unit-height control, while original per-case rows correctly
name all three engines. The table above uses those actual method names.
