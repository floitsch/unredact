# Pinned baseline source review

Latest expanded results and the real-document holdout appear at the end of this
document. Joint external results use an explicitly declared aggregation wrapper.

The [completed multidocument comparison](RESULTS.md) includes geometry and common-
language results on 192 held-out spans from eight documents, 192 absent controls, plus
768 pixelation cases. Selected Rust/T5 ranks 90/192 correct first versus Alex/T5's
46 and Unredactron/T5 CSV controls' 9, 12 and 47 at fixed scales. All missing-context
failures remain in the denominator. Native CSV controls are a declared adaptation;
Alex's paid Claude path was not run. Earlier small-case results below are
development history.

GitHub access was restored on 2026-10-19 using the updated execution permissions.
Ordinary sandbox Git requests still failed DNS; the same read-only request outside
the sandbox succeeded. Both repositories are now checked out locally and clean.

| Repository | Reviewed commit | Implementation actually inspected |
| --- | --- | --- |
| [Alex-Gilbert/unredact](https://github.com/Alex-Gilbert/unredact/tree/196cc322d980666782e23a6e26aba9712ca4c6d1) | `196cc322d980666782e23a6e26aba9712ca4c6d1` | Current browser JavaScript + Rust/WASM, plus older Python path |
| [mortenfj/unredactron](https://github.com/mortenfj/unredactron/tree/d00be632aa463355ec72158244e340310eba7253) | `d00be632aa463355ec72158244e340310eba7253` | Python font profiler and main candidate scoring loop |

The second repository is the matching project found for the original
“mortenfi/unredaction” reference. Alex's LICENSE is MIT. Unredactron's actual
LICENSE declares public-domain use; its README badges are inconsistent with it.
Adapters import/extract code from the pinned checkouts; our production solver
does not incorporate either implementation.

## Findings that change the comparison

**Kerning is already implemented in Alex's project.** Browser dictionary mode
measures whole strings with Canvas `measureText`, including adjacent context.
Its DFS width tables store pair-dependent advances. The older Python path also
uses pair-dependent Pillow measurements. A Rust rewrite or adding kerning alone
therefore does not establish an advantage. Relevant source:
[browser solver](https://github.com/Alex-Gilbert/unredact/blob/196cc322d980666782e23a6e26aba9712ca4c6d1/unredact/static/solver.js),
[WASM width table](https://github.com/Alex-Gilbert/unredact/blob/196cc322d980666782e23a6e26aba9712ca4c6d1/unredact-wasm/src/solver/width_table.rs).

**Learning from unredacted scans is also implemented in both.** Alex performs
pixel matching of rendered text against OCR-aligned visible text, masking the
redaction. It searches font family and size; marquee mode additionally optimizes
fractional size and position. We should not describe subpixel positioning itself
as unique to this project. Its automatic analysis needs an OCR line overlapping
the target; our earlier stand-alone-mask fixtures are unsuitable for measuring
that default workflow. Source:
[font detection](https://github.com/Alex-Gilbert/unredact/blob/196cc322d980666782e23a6e26aba9712ca4c6d1/unredact/static/font_detect.js),
[analysis](https://github.com/Alex-Gilbert/unredact/blob/196cc322d980666782e23a6e26aba9712ca4c6d1/unredact/static/main.js).

**Unredactron's calibration score is not held-out accuracy.** It selects one OCR
reference word, computes `scale_factor = actual_width / base_width` independently
for every tested font/size, then measures that same word again. With zero tracking,
its width residual is zero by construction, subject to floating-point rounding.
It uses a coarse height penalty and ranks these configurations by that score.
The kerning-mode label is derived from whether tracking is zero; this is not an
independent kerning-on/off experiment. The main loop uses Pillow's whole-string
width, scale, and per-character tracking. Source:
[font profiler](https://github.com/mortenfj/unredactron/blob/d00be632aa463355ec72158244e340310eba7253/font_profiler.py),
[candidate scoring](https://github.com/mortenfj/unredactron/blob/d00be632aa463355ec72158244e340310eba7253/unredactron.py).

Our plausible advantages remain multi-word calibration with independent audits,
retaining typography uncertainty, joint evidence across linked occurrences,
duplicate/dependence handling, native-PDF anchors, and mosaic-specific inference.
They must be measured separately. AI hooks and candidate dictionaries are useful
interfaces, but existing projects already support candidate input and Alex already
integrates contextual model assistance.

## Adapter fidelity and declared adaptations

`scripts/adapters/alex_browser.py` runs the current browser code in a fresh Chrome
context. It uses the original `ocrPage`, `analyzeRedaction`, `detectFontMasked`,
`measureWidths`, and compiled WASM `solve`. The two private functions are extracted
without body edits and exported in a generated work-directory module. OCR URLs
are redirected to pinned local Tesseract.js 5.1.1/core 5.1.1/English data assets.
No external model is called. Fonts are loaded in manifest order for determinism.
The supplied geometry replaces UI selection; the supplied vocabulary replaces a
dictionary upload. The current comparison does not exercise PDF.js or redaction
detection. Default effective tolerance is 1.1px; a separately named sensitivity
configuration uses 5.1px, the largest normal slider value plus upstream's 0.1px.

`scripts/adapters/morten_unredactron.py` calls `FontProfiler.profile_document`
with the original image and bundled font directory. It extracts the original
candidate loop from `analyze_pdf` as an AST and executes it without body changes,
bypassing PDF loading and rectangle detection because the track supplies images
and target geometry. A recording font proxy exposes each width; the adapter
reports the corresponding original scores and ties, and verifies its best group
agrees with upstream selection. CSV confidence is fixed to zero. Compatibility
is reported as unavailable because the main program does not return a complete
accepted-candidate set. The optional fidelity test compares the extracted loop
with the original `analyze_pdf` on identical supplied contours and candidates.

Both adapters reject multi-occurrence and unsupported evidence inputs explicitly.
Neither silently invents a joint inference method. The initial physical-scoring
comparisons disable AI; later sections explicitly identify local contextual
model configurations. External adapters still disable upstream AI assistance,
so those comparisons do not measure upstream language-model or interactive
manual-refinement quality.

## Contextual scan track

`scripts/prepare_external_comparison.py` generates independent Pillow raster
fixtures with visible text on both sides of every mask, blur, and noise. Candidate
names are synthetic. Every tool receives the same page, target box, vocabulary,
and padding-corrected width. The primary measurement sigma is fixed at 1.5px;
Alex's tolerance settings are fixed separately as above. No font, OCR, calibrated
profile, or truth label enters the adapter protocol.

This controlled track measures single-occurrence candidate ranking. Known padding,
single-font pages, supplied locations, limited font families, and repeated names
across fonts limit its generality. A narrow synthetic win would not establish
real-document identification accuracy. Truth-absent cases must remain visible in
the report, including wrong unique best ranks and unavailable acceptance decisions.

## First direct run: useful smoke test, not a broad accuracy claim

Artifacts: `runs/external-context-suite-001/suite.json` and
`runs/external-context-comparison-002/comparison.json`. Two synthetic names appear
in each of three fonts: six truth-present cases, plus six absent-name controls.
Every configuration completed all twelve cases. Both required external pins were
verified before/after execution; `missing_verified_external` is empty.

| Configuration | Unique correct first / 6 | Truth in best tie group / 6 | Truth compatible / 6 | Mean compatible candidates, truth present | Mean time per case |
| --- | ---: | ---: | ---: | ---: | ---: |
| Our single-occurrence solver | 2 | 2 | 6 | 71.5 | 1.46s |
| Alex default 1.1px | 0 | 1 | 3 | 13.0 | 5.38s |
| Alex wide 5.1px | 0 | 1 | 4 | 50.83 | 5.33s |
| Unredactron | 0 | 0 | Unavailable | Unavailable | 2.10s |

These acceptance rules are different; compare recall together with set size.
Our wider compatible sets retain truth more often but leave substantially more
ambiguity. Alex wins one direct counterexample: it puts truth in its best tie
group where our rank is ninth. Unredactron selected CENTURY.TTF and reported
100% calibration accuracy for all three actual font families, illustrating why
the fitted-reference score cannot be treated as independent identification accuracy.

On absent-name controls, wrong unique first ranks occur in 6/6 for ours, 4/6 for
each Alex configuration, and 3/6 for Unredactron. A wrong first rank is not an
assertion of identity. None of the tools exposing compatible sets returned an
empty set or a single wrong compatible candidate in these controls. Do not treat
Unredactron's unavailable compatible-set result as successful abstention.

The experiment supports a narrow unique-first ranking advantage on these six
cases. It also exposes ranking failures and cannot establish broad superiority.
No production scoring was tuned against these baseline results. Next comparisons
should expand fixed configurations and separately test joint observations, unknown
padding, absent fonts, and independently labeled real documents.

The preceding `external-context-comparison-001` attempt failed because the harness
dereferenced the Python virtualenv symlink before execution. This harness bug is
fixed and regression-tested. Those failures remain recorded and are excluded from
claims about upstream inference quality.


## Expanded fixed-configuration comparisons

No production solver or configuration tuning was applied between the initial and
expanded runs. `runs/external-context-comparison-003/comparison.json` records
192 successful executions, no failures, and verified external provenance. Eight
synthetic names recur across three font conditions (24 present and 24 absent
cases), with 400 candidates per present case. These are not 24 independent names.

| Configuration | Unique correct first | Truth compatible | Mean present survivors | Wrong unique first, truth absent |
| --- | --- | --- | --- | --- |
| Local single | 4/24 | 24/24 | 83.79 | 23/24 |
| Alex default | 0/24 | 5/24 | 15.42 | 17/24 |
| Alex wide | 0/24 | 16/24 | 62.25 | 17/24 |
| Unredactron | 0/24 | unavailable | unavailable | 17/24 |

Alex includes truth in its best tied group once. No compatible-set implementation
returns an empty set in the absent cases. Our improved recall retains substantial
ambiguity. Ranking first is not an asserted or calibrated identity probability.
A separate forced-kerning diagnostic improves one truth rank from ninth to second
but produces no additional unique first ranks; it did not change production code.

### Joint evidence extension

`external_joint.py` runs each original external adapter independently on each
occurrence, collects all raw candidate widths, and sums Gaussian-style distance
costs using the supplied measurement uncertainty. It deduplicates identical image
regions and explicit dependence keys. This is OUR aggregation, not an upstream
feature; it does not learn the external predictor's font/model uncertainty.

`runs/external-joint-comparison-001/comparison.json` records 48 successful runs:
eight present and eight absent cases, each combining the three font conditions.

| Configuration | Unique correct first | Truth compatible | Mean present survivors |
| --- | --- | --- | --- |
| Local joint | 6/8 | 8/8 | 32.125 |
| Alex measurements + our aggregation | 0/8 | 1/8 | 2.25 |
| Unredactron measurements + our aggregation | 0/8 | 0/8 | 12.625 |

The external aggregation can amplify model mismatch: Alex's extension produces
two single-wrong-compatible executions across both truth conditions. Empty absent
sets (Alex 5/8, Unredactron 3/8) must be interpreted with poor truth retention.
Local joint still ranks a wrong candidate uniquely first in all eight absent cases
and never returns an empty compatible set. Links and mask padding are supplied.
These results support joint measurements under this controlled setup, not corpus
linking accuracy or recovery of original real-world redactions.

### Existing-document ordinary-word holdout

`runs/external-document-comparison-001/comparison.json` records 32 successful runs
on four visually checked, already-visible ordinary words from page 1 of the
baseline repository's EFTA00037366.pdf. All four words are synthetically masked;
original redactions remain untouched and unlabeled. The source's authenticity was
not independently established. Present vocabularies contain 400 ordinary words.

| Configuration | Unique correct first | Truth compatible | Mean present survivors |
| --- | --- | --- | --- |
| Local single | 1/4 | 4/4 | 94 |
| Alex default | 0/4 | 1/4 | 4.25 |
| Alex wide | 0/4 | 3/4 | 28.25 |
| Unredactron | 0/4 | unavailable | unavailable |

Alex default returns no compatible candidates for both present and absent versions
of the same case, so its one empty absent set is not evidence of effective
missing-truth detection. Local single ranks a wrong candidate uniquely first in
all four absent cases. Four words on one page are a small diagnostic, not a general
accuracy estimate. Joint and document comparisons overlapped in execution; their
latencies should not be compared as isolated speed measurements.

All three runs report `missing_verified_external: []`. Six adapter tests pass,
including upstream-loop fidelity, joint aggregation, and duplicate handling.


## Optional scan space calibration ablation

`runs/context-space-ablation-001/comparison.json` compares the current binary with
and without `--learn-spaces` on the unchanged expanded contextual suite (SHA-256
`5865a2e93b607d8f124874b157ef1635ce517993b2b902605718fcf14bcc815f`). All 96
executions succeed. Unique correct first increases 4/24 to 5/24; truth compatibility
remains 24/24. Two cases gain a unique correct first rank and one loses it, so
the gain is not uniform. Mean present compatible-set size increases from 83.79 to 86.79.
Both configurations uniquely rank a wrong candidate first in 23/24 absent cases,
with no empty compatible sets. This is a local feature ablation, not a rerun of
external baselines. Their earlier frozen results remain separately scoped.

`runs/senate-spacing-002` audits visible OCR-labelled pairs on six predeclared
400-DPI scan pages, using extracted source images. Among eight block profiles with
sufficient pair audits, two primary models improve and six remain unchanged; the
other three successful block profiles lack enough audit pairs. Two improvements
are 4.8722 -> 4.1008 pixels (18 audit pairs) and 3.8550 -> 3.5877 (10 audit pairs).
All six whole-page primary models decline a spacing correction. Five additional
blocks cannot calibrate because they have only three distinct validation words.
Mixed typography, unknown fonts, and scan distortion remain larger open problems.
This is within-page width prediction using uncorrected OCR labels, not original
redaction recovery or held-out-page generalization. Runtime overlapped the local
ranking ablation and should not support a speed comparison.


## Typography size grouping diagnostic

`group-typography` and `calibrate --size-group` pool lines of similar normalized
height across OCR blocks. Only training words determine size membership. This
separates size bands; same-size font/style mixtures remain unresolved.

The diagnostic compares the same visible audit-word observations under each
model, using the first fit selected by word validation. No spacing adjustment is
applied in this experiment. It does not score original redactions.

| Page set | Audit observations | Whole-page RMSE | Group RMSE | Shared block observations | Block RMSE on shared set | Group RMSE on shared set |
| --- | --- | --- | --- | --- | --- | --- |
| Development: 34,35,36,100,136,194 | 115 | 13.52 px | 2.79 px | 68 | 2.73 px | 2.33 px |
| Additional, frozen rule: 48,64,120,208,300,400 | 148 | 16.17 px | 4.93 px | 102 | 2.74 px | 2.60 px |

Artifacts: `runs/senate-typography-001` and
`runs/senate-typography-holdout-001`. The additional pages use fresh source-image
extraction and OCR; each page fits its own visible training words. Grouping beats
the matched whole-page model on all twelve pages, but loses against available
OCR-block fits on two development and three additional pages. Sparse lines remain
unassigned; one development group lacks enough validation words. Excluded regions
are reported, so these results do not establish full-page coverage. Repeated words
and within-page observations are correlated. Labels are uncorrected OCR output;
this measures width prediction, not unknown-redaction accuracy or external-tool
superiority. Sixty Rust tests and all-target Clippy pass.

### Optional source-raster metric hypotheses

The Rust app now optionally compares its default design metrics with FreeType
normal hinted metrics at explicit ppem values. The frozen experimental grid is
12,16,24,48,96,192; scale/tracking/bias and conditional residual corrections are
fitted as before. Modes are selected by validation, not audit. This does not
reproduce Word's layout engines, and ppem selection does not identify the original
printer/screen resolution. Pixelated-image synthesis still uses design metrics.

On the same size-grouped Senate audit observations, expanded search worsens
pooled RMSE from 2.786 to 2.958 px (115 words) and 4.932 to 4.975 px (148 words).
It improves five of twelve pages. The option remains experimental and off by
default. Results: `runs/senate-measurement-002` and
`runs/senate-measurement-holdout-002`. Both use cached uncorrected OCR; correlated
width observations are not independent recovery trials.

A separate fresh-OCR local ablation on the earlier four deliberately masked
ordinary document words improves unique correct first from 1/4 to 2/4 while
truth compatibility remains 4/4. Both methods retain compatible alternatives
when truth is absent. All sixteen executions succeed. This uses the same inputs
as the earlier document comparison, but reruns only our configurations; it is not
a new head-to-head external evaluation. Artifact:
`runs/measurement-document-ablation-001`.

New profiles fingerprint exact font bytes; all native consumers use immutable
snapshots and fail on mismatches during later scoring. This improves reproducibility,
not measured recovery accuracy. Installed-library/source identity is inventoried
separately. All 65 Rust tests and all-target Clippy pass.

The larger local synthetic scan ablation (`runs/measurement-context-ablation-001`)
finishes 96/96 executions. Expanded mode search reduces unique correct first
from 4/24 to 2/24; both retain truth in the compatible set for 24/24 present cases,
and neither rejects the entire vocabulary in any of 24 absent cases. Mean time
increases from 1.54 to 2.38 seconds. This contradicts a general improvement claim
and supports retaining the option as an explicit experiment only. No mode-grid
or selection-rule tuning followed these audit/ranking results.

### Neural font proposal integration and controls

Rust calibration can now construct training-only word montages and invoke the
local font CNN. Eight proposed font files supplement existing candidates; model
similarities do not alter redaction ranking. Physical validation selects fits,
audit words remain withheld, and source crop/font/model hashes are retained.
This does not split mixed-font regions automatically.

A full-catalog control is necessary: merely adding bold/italic font files can
improve results without a learned image model. On Senate scan audit sets,
default/neural/full-catalog RMSE is 2.786/2.820/2.981 pixels (115 words) and
4.932/4.932/4.932 pixels (148 words). Thus no real-scan improvement over default
is established. Artifacts: `runs/senate-neural-calibration-001` and
`runs/senate-neural-calibration-holdout-001`.

On four synthetic bold/italic pages (two families excluded from network training),
NN top-eight suggestions contain the exact fonts at ranks 1,1,1,2; physical fitting
selects those files. Candidate ranking improves from default 0/16 unique correct
first to 3/16 with NN proposals, but full-catalog search also reaches 3/16 and is
faster (1.62 versus 4.60 seconds mean). All arms retain truth in 16/16 present
compatible sets and retain alternatives in all absent cases. All 96 executions
succeed. The controlled synthetic result supports font coverage, not a separate
neural superiority claim. Artifacts: `runs/neural-style-suite-002` and
`runs/neural-style-comparison-001`. External tools were not rerun in this ablation.
All 69 Rust tests and all-target Clippy pass.

### Styled-font comparison with matched-catalog controls

The same `neural-style-suite-002` inputs were subsequently run against both pinned
external implementations. There are 16 present-vocabulary and 16 matched
absent-vocabulary cases per configuration, four fonts/styles, and 400 candidates.
These cases reuse four synthetic identities across fonts and are correlated.
No production scoring or neural weights were changed for these comparisons.

Alex's bundled manifest has five regular faces, but its application already
supports user-uploaded fonts (`loadFonts` and `onFontAdded` in `main.js`). Font
upload support is therefore not a differentiator. We added separately labelled
catalog controls supplying both tools the same 37 distinct font files used by
our full-catalog arm. The Alex wrapper replaces served font assets/manifest;
the Unredactron wrapper changes the original profiler's font directory. Neither
changes upstream fitting or scoring code. These are configurations of the
upstream algorithms, not claims about their bundled defaults or expert tuning.

| Configuration | Unique correct first | Truth compatible | Mean compatible candidates | Empty set when truth absent |
| --- | --- | --- | --- | --- |
| Our default fonts | 0/16 | 16/16 | — | 0/16 |
| Our full catalog | 3/16 | 16/16 | 97.63 | 0/16 |
| Our neural proposals | 3/16 | 16/16 | — | 0/16 |
| Alex bundled, 1.1px | 1/16 | 6/16 | 37.50 | 1/16 |
| Alex bundled, 5.1px | 1/16 | 10/16 | 76.56 | 0/16 |
| Alex matched catalog, 1.1px | 0/16 | 3/16 | 19.75 | 3/16 |
| Alex matched catalog, 5.1px | 0/16 | 11/16 | 78.38 | 0/16 |
| Unredactron bundled | 0/16 | not exposed | not exposed | not exposed |
| Unredactron matched catalog | 0/16 | not exposed | not exposed | not exposed |

All 224 new primary executions succeed: 128 bundled-external/full-catalog runs,
64 Alex matched-catalog runs, and 32 Unredactron matched-catalog runs. Our 32
repeated full-catalog results match the preceding local ablation exactly; they
are consistency checks, not additional independent trials. The merged report
checks suite hashes, per-case input hashes, coverage, and repeated outputs:
`runs/style-catalog-control-001/combined-summary.json`. Source reports are
`runs/neural-style-comparison-001/comparison.json`,
`runs/external-style-comparison-001/comparison.json`,
`runs/external-style-catalog-comparison-001/comparison.json`, and
`runs/external-style-morten-catalog-002/comparison.json`.

Our higher unique-first count coexists with 13/16 wrong unique first ranks and
large compatible sets. Alex bundled puts truth in its best tied group in 4/16
or 5/16 cases, depending on tolerance; our full-catalog arm does so in 3/16.
Different tie handling and rejection rates matter. Empty absent-case sets must
also be interpreted alongside present-case truth loss. This is not evidence of
reliable recovery of unknown redactions or broad superiority.

The measurement convention is a material limitation. These fixtures supply
padding-corrected raster ink bounds; Alex's Canvas `.width` and Unredactron's
Pillow `.getlength()` predict text advance. With oracle font/size information,
advance minus ink width ranges from -1.844 to +4.656 pixels here, with median
absolute difference 1.602 pixels. This diagnostic is saved in
`runs/style-catalog-control-001/ink-advance-diagnostic.json` and was never passed
to adapters. Results apply to this ink-mask track; an independently declared
advance/visible-anchor track remains necessary.

The initial Unredactron catalog pilot used hash-based font filenames. Since the
upstream profiler sorts filenames, we preserved that pilot at
`runs/external-style-morten-catalog-001` and reran with original basenames in
`-002`. Both score 0/16 unique correct first, but tie outcomes differ. Only the
basename-preserving run is included above. Its old wrapper is preserved as
`runs/style-catalog-control-001/morten_catalog_hash_names.py`.

Mean runtimes in these overlapping runs are 1.69s for our full catalog,
5.90/5.89s for Alex bundled, 9.26/9.24s for Alex matched, and 2.20/2.45s for
Unredactron bundled/matched. They are descriptive measurements under contention,
not isolated performance benchmarks. Three new Python tests pass for catalog
integrity and report matching; seven existing local-LLM provider tests also pass.

### Advance geometry, mask-aware OCR and visible scan anchors

`runs/scan-anchor-suite-001` declares a separate advance-width track: the same
four styled fonts, four fresh synthetic names selected with seed 9467, natural
right-context placement, and fractional Pillow advance supplied identically to
all tools. There are 16 present and 16 absent cases per configuration. Fonts,
labels and OCR text remain withheld. Only Alex's fixed wide 5.1px configuration
is rerun here, with both bundled fonts and the shared 37-file catalog.

Rust now supports `scan_span`: shape a candidate together with the visible OCR
words immediately before/after it, and compare the outer ink endpoints. Mask
width does not enter that score. Ordinary spaces on both sides and uniform
typography are explicit assumptions. `refresh-ocr` first blanks known/detected
bars in an auxiliary image, then refines recognized word bounds on original
pixels. The optional learned-space model is unchanged from earlier work.

| Configuration | Unique correct first | Truth in best group | Truth compatible | Mean compatible candidates |
| --- | --- | --- | --- | --- |
| Our full catalog, width, mask-aware OCR | 0/16 | 1/16 | 16/16 | 78.44 |
| Our full catalog, anchors, mask-aware OCR | 0/16 | 4/16 | 16/16 | 86.63 |
| Our full catalog, anchors + spaces, mask-aware OCR | 3/16 | 7/16 | 16/16 | 129.50 |
| Alex bundled, wide | 0/16 | 4/16 | 12/16 | 63.19 |
| Alex matched catalog, wide | 2/16 | 7/16 | 14/16 | 63.31 |
| Unredactron matched catalog | 0/16 | 0/16 | not exposed | not exposed |

All 192 primary executions complete successfully, in
`runs/scan-anchor-local-002/comparison.json` and
`runs/scan-anchor-external-001/comparison.json`; the matched-input summary is
`runs/scan-anchor-suite-001/combined-summary.json`. No implementation with a
compatible-set output rejects the entire absent-truth vocabulary. Both our
anchors-plus-spaces configuration and Alex matched catalog give a wrong unique
first rank in 8/16 present cases. The one-case unique-first difference is not
strong evidence of general superiority, especially with correlated cases and
large compatible sets. This is not an original-redaction accuracy estimate.

The initial local run is preserved at `runs/scan-anchor-local-001`. Standard OCR
merged some visible words into mask tokens; filtering those tokens then removed
the visible neighbors. In one case anchors became “correspondence” and “the”,
omitting visible “identifies” and “as”. Its anchor arms had ten failed executions
each and retained no truths in the eleven successful present cases. These are
implementation failures, not useful abstentions. Another failure exposed an
invalid raw-height check: “identifies” and “as” can have very different ink heights
in the same font. That check was removed; row overlap remains required.

The final implementation additionally checks imported image identity and rejects
four or more dark pixels in either gap between anchors and mask. This detects
unrecognized intervening ink before target registration, solve or LLM search.
The check is conservative and may reject noise; it does not prove OCR spelling.
The post-run audit rejects all 44 completed synthetic anchor executions from the
initial run and accepts all 64 from the revised run. Four completed real-document
anchor executions also pass in each version. Pixel audit artifacts live under
the respective local run directories. Earlier executable snapshots are retained
as `unredact-v1` and `unredact-v2` there.

The existing real-document controlled-mask suite was rerun as
`runs/scan-anchor-document-001` and `-002`. In both, width scoring completes all
eight executions and gives 1/4 correct unique first. Each anchor arm completes
only two of eight executions (one present and its absent counterpart), with
0/4 correct unique first over the intended present cases. The other three targets
have unreliable neighboring OCR and are counted as failures. Mask-aware OCR
does not solve that problem, so no real-scan ranking improvement is established.

The pixel guard was added after the revised comparison. Its validation-only
effect is audited by replaying every successful revised local/document solve on
the saved OCR and profiles with the final binary; this is a consistency check,
not additional independent trials. The replay report is
`runs/scan-anchor-local-002/final-binary-replay.json`: all 108 successful saved
solves reproduce exactly. The earlier v2 auxiliary
OCR metadata incorrectly described psm 6; actual execution used the existing
Tesseract psm 3 command throughout. Final metadata records psm 3 correctly.
All 74 Rust tests and all-target Clippy pass.

### Scanned bar fringes, false proposals and additional real pages

The four-word development document exposed two different OCR problems. Blanking
only a detected bar's black core left gray outlines, which Tesseract read as
brackets joined to adjacent words. The auxiliary OCR image now includes dense
contiguous fringes, capped at eight pixels per side and stopped by white gaps.
Original image pixels remain unchanged. Separately, high-resolution glyph parts
were proposed as bars: one 25×6-pixel fragment occurred amid roughly 50-pixel-high
text. A local-height filter now rejects such small proposals when at least three
nearby reliable words support that decision. Explicit masks bypass this filter;
raw and retained automatic proposals are recorded.

On eight manually reviewed visible neighbors in the development document,
default OCR improves from 5/8 exact transcriptions to 8/8 after fringe removal;
high-confidence correct observations improve from 4/8 to 6/8. The optional
English `tessdata_best` model also reads 8/8 exactly and accepts 7/8 at the unchanged
85-confidence threshold. This is a small development diagnostic, not a held-out
OCR accuracy estimate. The corresponding candidate runs are
`runs/scan-anchor-document-003` and `-004`: default anchors now complete two of
four intended present targets, the larger model three of four, but neither gives
a correct unique first rank. Width ranking remains 1/4.

The optional model comes from
[tessdata_best](https://github.com/tesseract-ocr/tessdata_best/tree/e12c65a915945e4c28e237a9b52bc4a8f39a0cec),
commit `e12c65a915945e4c28e237a9b52bc4a8f39a0cec`, under Apache-2.0. English weights
SHA-256: `8280aed0782fe27257a68ea10fe7ef324ca0f8d85bd2fd145d1c2b560bcb66ba`.
`refresh-ocr --tessdata` explicitly selects English LSTM recognition, verifies the
model hash before/after OCR and records it in a separate artifact. System OCR
remains the default. [Tesseract's documentation](https://tesseract-ocr.github.io/tessdoc/Data-Files.html)
describes the model sets and their speed/accuracy tradeoffs.

The first model diagnostic (`ocr-anchor-diagnostic-001`) is invalid: a model-only
directory lacks the `tsv` config file, and Tesseract returned ordinary text with
exit code zero. `INVALID.json` records the error. The corrected diagnostic and
production hook request TSV with a parameter and validate its header. Valid
artifacts are `ocr-anchor-diagnostic-002`, `ocr-anchor-diagnostic-default-001`,
`ocr-fringe-default-001`, and `ocr-fringe-lstm-001`.

Four new Senate source pages, 90/160/260/440, supplied twelve manually reviewed
ordinary visible words. Fixed body crops preserve original scan pixels; all
evaluation words are masked on each input, original redactions stay untouched,
and the shared vocabulary has 400 entries with matched absent controls. Source
hashes, crops, labels and review records are in `real-scan-holdout-preparation-001`
and `real-scan-holdout-suite-002`. Preparation `-001` stopped because “attended”
appeared twice; the reviewed first occurrence was specified before scoring.
The second occurrence remains visible and is a training word in the local
profile. All tools receive those same pixels. The target holdout is therefore
by selected occurrence, not lexical identity; `repeated-word-audit.json` records
this limitation. It must not be described as a test on entirely unseen words.
These pages exposed the false-proposal bug, so the corrected rerun is development
evidence despite the directory names. It requires another untouched evaluation
before a generalization claim.

| Configuration | Successful executions | Unique correct first (12 intended present cases) | Truth compatible | Mean compatible candidates |
| --- | --- | --- | --- | --- |
| Historical width scorer, before fringe changes | 24/24 | 2/12 | 12/12 | — |
| Current width scorer | 24/24 | 1/12 | 12/12 | 44.33 |
| Current anchors + spaces, default OCR | 24/24 | 0/12 | 12/12 | 131.25 |
| Current anchors + spaces, larger OCR model | 24/24 | 0/12 | 12/12 | 133.17 |
| Alex bundled, wide | 24/24 | 1/12 | 6/12 | 18.50 |
| Alex matched catalog, wide | 24/24 | 0/12 | 5/12 | 18.25 |
| Unredactron matched catalog | 24/24 | 0/12 | not exposed | not exposed |

Before local-height filtering, each anchor arm failed four of 24 executions and
had 0/12 correct unique first overall. Filtering removes those failures but does
not improve candidate ranking. Every compatible-set configuration retains
alternatives in all absent cases. Current anchors put a wrong candidate uniquely
first in all twelve present cases; compatibility is not identification.

Reports: `real-scan-holdout-local-001/comparison.json`,
`real-scan-holdout-local-002/comparison.json`, and
`real-scan-holdout-external-001/comparison.json`. The merged summary verifies
shared suite/input hashes and coverage. All 144 executions in the final local
and external reports succeed. The historical width control uses the saved v2
binary; the pre-filter current binary is also preserved with its run.

The detector repair preserves 110 more OCR words across these four crops. On
38 identical withheld OCR observations, the selected profiles' pooled width
RMSE changes from 3.707 to 2.349 pixels. The gain is concentrated on page 90;
the other three pages are mixed. These are OCR-derived labels and correlated
within-page observations. `audit_profile_pairs` checks identical image identity,
text, bounds and membership in both audit sets; results are saved in
`real-scan-holdout-suite-002/matched-word-audit.json`. Its comparison avoids
mistaking a changed set of recognized words for improved measurement accuracy.

All 76 Rust tests, all-target Clippy and release builds pass. These changes
improve OCR coverage and parts of calibration, but the ranking results do not
establish superiority over the external tools. Phrase spacing, typography
variation, contextual ranking and genuinely independent evaluation remain work.

The earlier synthetic advance suite was rerun after the detector changes in
`runs/scan-anchor-local-003`. All 96 executions succeed, and all adapter outputs
and complete solve reports match the preceding `-002` run exactly on identical
input hashes. This rules out a regression on that specific suite; it is a
consistency check, not additional independent accuracy evidence.

## Target-row spacing experiment

`runs/line-spacing-001` tests the optional `--scan-anchors --line-spaces` model.
It learns an additional per-space offset from visible training pairs on each
target row; candidate labels are absent from estimation. The rules were fixed
before these ranking runs. Estimates require three distinct high-confidence
pairs, exclude held-out words and masks, and retain existing profile uncertainty
while adding local scatter. Model assumptions and sample coordinates appear in
each solve report. Two of twelve real targets lacked sufficient pairs for the
selected fit and used the profile-wide fallback.

On the existing twelve-word real-scan development suite, both arms completed
all 24 present/absent executions. Unique correct first rank improves from 0/12
to 1/12, equal to the existing width scorer and Alex bundled wide configuration
on these inputs. All twelve truths remain compatible, but mean compatible-set
size increases from 131.25 to 144.50 of 400. Neither arm rejects any absent
vocabulary. Mean lower bound of truth rank changes from 16.33 to 14.08;
individual words improve and regress. The selected-fit truth-span RMSE changes
from 5.609 to 5.342 pixels. Fit selection can change between arms, so this last
number is a post-run scoring diagnostic, not a controlled fixed-fit error test.

`real-combined.json` joins the previous external results only after verifying
suite/per-case hashes, equal coverage and identical repeated control outputs.
External tools were not rerun for this optional local change. `real-diagnostic.json`
contains the per-word results, evaluation-label provenance and selected pair
estimates. These pages are already development data; no new original redaction
has a truth label, and this does not establish general recovery accuracy.

With the feature disabled, all 168 saved solve reports from the final earlier
real and synthetic local comparisons reproduce exactly (`default-replay.json`).
All 80 Rust tests and all-target Clippy pass.

On the reused sixteen-case synthetic advance track, both arms completed all
32 present/absent executions. Local line spacing regresses unique correct first
from 3/16 to 1/16 and truth in the best tied group from 7/16 to 5/16. Compatible
sets increase from 129.50 to 133.69/400 on average; both retain all sixteen
truths and neither rejects an absent vocabulary. All sixteen cases have at least
one non-fallback fit. The unchanged control matches the earlier run exactly;
`synthetic-combined.json` verifies shared inputs and compares the pinned external
results (Alex matched wide: 2/16 unique correct first). Thus this option provides
no consistent ranking improvement and stays disabled by default. These reused
cases are development evidence, not additional independent trials.

## Offline contextual ranking

`runs/context-model-001` adds a separately labeled AI-assisted configuration.
It first computes the ordinary full-catalog width report, then ranks its
physically compatible candidates with public BERT base uncased weights pinned
to revision `86b5e0934494bd15c9632b12f734a8a67f723594`. Original physical
scores, ranks, compatibility and ambiguity are preserved verbatim. The adapter
puts incompatible candidates afterward in their original physical order.

The model scores candidate WordPieces by initially masking all of them and
revealing them left-to-right with both visible contexts. No later candidate
token is visible when scoring an earlier token. Average token log score is
averaged again over independent occurrences. This length-conditioned heuristic
is not a normalized posterior over hidden text. Input context comes from up to
twelve visible OCR words on each side of the row, stopping at masks, unreliable
words or large gaps. Punctuation and provenance are retained. No candidate label
is supplied to the model. Hashes, requests, responses, options and the exact
Python environment are preserved; inference is offline and requires no token.

On the existing twelve-word Senate development suite, correct unique first
ranks improve from 1/12 width-only to 7/12 contextual. All 48 executions succeed;
compatible sets are unchanged and neither mode rejects the absent vocabulary.
`development-combined.json` verifies repeated controls and shared inputs against
the pinned earlier external results (Alex bundled wide 1/12, matched wide 0/12,
Unredactron matched 0/12). This adds language knowledge; it is not a claim that
physical measurement became more accurate.

A fresh suite uses previously unused archive pages 35,120,350,500, chosen before
reading or scoring. Three manually reviewed ordinary words per page cover
varied grammatical roles. The 300-candidate pool samples mixed-role OCR words
from previous development images rather than using only noun distractors. All
selected words are masked in every supplied image. No unselected exact OCR
repeat of a target label was found in its crop. The initial preparation stopped
on OCR `feclings` for selected `feelings`; full-resolution review confirmed the
label and an explicit locator retained the original word. The incomplete
suite-001 and source/review artifacts are retained; suite-002 is evaluated.

The frozen implementation scores 7/12 on these fresh pages versus 0/12 width-only,
with 48/48 local executions successful. All truths remain compatible; mean
compatible-set size is 34.75/300. Every paired set is exactly identical between
modes. Neither rejects any absent vocabulary. This is still a conditional test
with known padding-corrected ink widths and selected readable words from one
archived report. Holding out pages from our development cannot exclude public
text exposure during BERT pretraining.

An evaluation-only control scores all candidates before filtering: correct
context alone scores 5/12, adding the physical filter gives 7/12, and cyclically
shuffled contexts give 0/12 with or without that filter. All twelve original
hybrid truth ranks reproduce exactly. `holdout-context-ablation.json` includes
complete scores, context substitutions and hashes. No parameters were fitted
in this control, and these are repeated cases rather than additional trials.

On the earlier four-word EFTA development document, contextual ranking regresses
from 1/4 to 0/4, with all 16 executions successful. Truth stays compatible and
missing-vocabulary rejection remains absent. Previous external results are
joined in `efta-development-combined.json` after checking inputs: bundled Alex
and Unredactron configurations also score 0/4. The EFTA source was copied from
the baseline repository; authenticity has not been independently established.

Failure diagnostics expose missing cross-line context and WordPiece-length
bias. Exploratory ranking by unnormalized score sums instead of implemented
means gives 11/12 development, 6/12 fresh Senate and 2/4 EFTA, versus 7/12,7/12,
0/4 for the implemented means. That post-hoc calculation is not a validated
configuration or an independent test; no scoring code changed after seeing it.
The contextual mode remains explicit. Broad superiority and reliable recovery
of unknown original redactions remain unproven.

The fresh external comparison has completed all 72 executions successfully.
`holdout-combined.json` verifies identical suite/input hashes and equal coverage
across the five configurations:

| Configuration | Correct unique first | Truth in best tied group | Truth compatible |
| --- | --- | --- | --- |
| Local width + contextual BERT | 7/12 | 7/12 | 12/12 |
| Local width only | 0/12 | 0/12 | 12/12 |
| Alex bundled, wide tolerance | 0/12 | 1/12 | 11/12 |
| Alex matched catalog, wide tolerance | 0/12 | 1/12 | 7/12 |
| Unredactron matched catalog | 1/12 | 1/12 | Not exposed |

Every configuration exposing compatibility retains candidates when truth is
absent. The model scores use language priors in addition to geometry; the
external rows use the pinned physical-scoring configurations described earlier.
This is a measured advantage on this selected scan task, with a contradictory
EFTA result and no original-redaction ground truth. The exact measured binary
is saved as `runs/context-model-001/unredact-benchmark`; final integrity audit
confirms frozen artifacts remained unchanged through all runs. Across the
development, fresh-local, fresh-external and EFTA comparisons, all 184 adapter
executions succeeded.


## Paragraph context and word normalization

`runs/context-paragraph-001` evaluates two separately enabled options. Paragraph
context follows OCR paragraph/line IDs only when text height, line pitch,
column boundaries and gutters agree; masks, low-confidence words and unresolved
line-end hyphens stop it. Word normalization divides summed token log scores by
the selected replacement's whitespace word count instead of its WordPiece count.
Neither changes physical fits or compatible sets. The four combinations were
fixed before testing a new document; all remain experimental.

Previously evaluated Senate pages and EFTA cases are now development data:

| Context / normalization | Senate correct unique first | EFTA correct unique first |
| --- | --- | --- |
| Same row / token (unchanged default) | 7/12 | 0/4 |
| Paragraph / token | 7/12 | 0/4 |
| Same row / word | 6/12 | 2/4 |
| Paragraph / word | 7/12 | 2/4 |

All 128 executions succeed. The unchanged default exactly reproduces prior
rankings and metrics; joined reports check identical suite/input hashes and
coverage. Compatible sets are identical across the four configurations in all
32 present/absent cases. No absent vocabulary is rejected. Paragraph context
recovers `feelings` but loses `family` in token mode; word normalization loses
`bipartisan`. These results do not justify replacing the default.

The new source is NASA's original [Rogers Commission volume 1 scan](https://ntrs.nasa.gov/citations/19860015255),
NTRS 19860015255. Its extracted one-bit 300-DPI image pixels are retained without
resampling. Pages 30,70,110,150 were preselected. Page 30 and the next five pages
are figures/captions and were excluded under the recorded screening rule; page
70 is charts, so the following prose page 71 is used. Fixed body crops retain
both columns. Nine ordinary-word labels on pages 71,110,150 were chosen before
scoring. The 300-candidate vocabulary uses fixed non-target pages
10,50,90,130,170,210 for distractors. Known padding-corrected ink widths and
supplied locations make this a candidate-ranking test, not detection accuracy.

Full-resolution visual review retains selected `relationship` despite OCR
`relatiqnship`. Explicit reviewed trims exclude punctuation after `breakdown`
and `configuration`, and adjacent-line ink in two OCR boxes. The final overlay
is `runs/nasa-holdout-suite-003/reviewed-targets.png`. Suite-001 failed preparation;
suite-002 failed visual box review; neither was scored. Suite-003, its vocabulary,
and all scoring options were frozen before comparison. A normalized OCR repeat
audit finds no unselected exact target-word repeats, subject to OCR errors.
Public pretraining exposure remains unexcluded. Screened EFTA00513855 pages are
contact tables, so that document was not used for this prose-continuation test.


All 144 fresh NASA executions complete successfully. `nasa-combined.json` checks
identical suite/input hashes and equal coverage for all eight configurations:

| Configuration | Correct unique first | Truth in best tied group | Truth compatible |
| --- | --- | --- | --- |
| Local width only | 4/9 | 4/9 | 9/9 |
| Local width + same-row/token context | 4/9 | 4/9 | 9/9 |
| Local width + paragraph/token context | 4/9 | 4/9 | 9/9 |
| Local width + same-row/word context | 5/9 | 5/9 | 9/9 |
| Local width + paragraph/word context | 7/9 | 7/9 | 9/9 |
| Alex bundled, wide tolerance | 0/9 | 0/9 | 2/9 |
| Alex matched catalog, wide tolerance | 0/9 | 0/9 | 1/9 |
| Unredactron matched catalog | 0/9 | 0/9 | Not exposed |

Every local configuration has the same compatible sets, averaging 23.44/300
candidates with truth present. The full physical reports agree exactly across
context modes on all 50 cases across the three documents. The combined option
improves `parallel`, `significance` and `particular` relative to width-only;
`transient` and `breakdown` still rank second and sixth. This independent-document
result supports the combined option, while the development regressions argue
against declaring a universally better normalization or context extractor.

No configuration exposing compatibility rejects an absent vocabulary. One local
case, with `memorandum` removed, has exactly one wrong compatible candidate.
All nine local absent cases rank a wrong candidate uniquely first. Thus neither
first rank nor sole physical compatibility establishes identity or proves that
the true text is in the list. The external AI features remain disabled; this
comparison does not establish superiority over their full AI-assisted workflows.
The words share three pages in one public document and are not nine independent
document trials. There is still no verified recovery of unknown original masks.

`benchmark-integrity.json` verifies all frozen runtime/source hashes after
completion. The exact measured binary is `runs/context-paragraph-001/unredact-frozen`,
with source snapshot beside it. After benchmarks completed, a validation-only
fix reapplies the existing 4096-byte context limit after paragraph expansion,
with a regression test covering both sides and multibyte OCR text. All benchmark
context sides are at most 92 UTF-8 bytes, so none crosses the new guard. The
measured binary remains preserved separately from that subsequent release build.


## Open candidate expansion: development results

`expand` now generates additional text from visible context and checks every
proposal against all independent linked observations. It preserves original
candidate IDs, aliases and metadata, saves both initial and expanded physical
reports, and emits a reusable candidate file. The protocol supports the pinned
local BERT weights and installed Llama/Gemma models through Ollama. Proposal
requests do not include the initial candidate vocabulary or truth labels.

`runs/expansion-001` deliberately removes each known true word from the vocabulary
on the existing controlled NASA, Senate and EFTA examples. These are reused
**development cases**, not fresh holdouts. Saved source projects and calibration
are unchanged; every initial physical report exactly matches its saved control.
Expanded lists are ranked with the existing paragraph/word BERT configuration.

| Provider configuration | Document | Omitted truth proposed and compatible | Correct unique first after expansion |
| --- | --- | --- | --- |
| BERT, 256 single-token alternatives + beam 16 at 2–6 pieces | NASA | 6/9 | 0/9 |
| Same BERT configuration | Senate | 8/12 | 4/12 |
| Same BERT configuration | EFTA | 1/4 | 0/4 |
| Llama 3.2 3B, up to 16 proposals | NASA | 0/9 | 0/9 |
| Gemma 3 4B, up to 16 proposals | NASA | 1/9 | 1/9 |

All 43 expansion-and-ranking cases complete successfully. BERT permits up to
three whitespace words and balances hypotheses across piece counts; the engine
caps proposals at 512. Llama/Gemma use temperature 0.7 and seed 42. Budgets differ,
so the table is not an equal-budget model comparison. Expanded vocabularies
also differ from the preceding external benchmark; its matched superiority
claims cannot be transferred to this open-generation track.

Gemma proposes `memorandum` and ranks it first, changing the previous one-wrong-
survivor case into two compatible candidates. BERT also proposes it, but ranks
it 18th in a much larger set. Thus expansion repairs that closed-list failure
under one measured configuration, without solving missing-vocabulary inference
generally. BERT's initial narrow-beam smoke run missed the word entirely and is
retained separately; broadening cheap single-token coverage was a development
change before the full evaluations were frozen.

Longer BERT beams sometimes generate awkward or nonlexical strings despite
canonical tokenization. Larger candidate sets introduce plausible distractors,
and word-mean scores can favor multiword text. A post-hoc raw-token-sum diagnostic
on NASA improves correct first from 0/9 to 1/9 (`configuration`) and changes
`memorandum` from 18th to 13th; production scoring was not changed. Neither score
is a calibrated posterior over arbitrary replacement lengths.

Llama's nine exact misses are also misses after case folding. Traces often show
repeated context or attempted correction of a truncated OCR fragment instead of
the marked gap. A prior known-answer temperature control at temperature 0/count 8
finds `thermometer` with both models through the older text-list response format;
through the new attributed-proposal format, Llama returns it and Gemma returns
modified phrases such as `digital thermometer`. Prompt wording and schema both
differ, so this does not isolate a schema-only effect. It motivates controlled
prompt simplification and measured-width feedback before further accuracy claims.

All 90 Rust tests and 15 Python provider tests pass, as do formatting and all-target
Clippy. The measured binary and source snapshots are preserved. After evaluation,
a boundary fix restricts automatic spaces to OCR-derived contexts, preserving
explicit user-supplied within-word boundaries. All 18 measured Llama/Gemma scan
prompts are verified identical under that fix. `final-integrity.json` records the
source hashes, model configurations, completed checks and this post-run change.
The local evaluation server was shut down after the runs.

## Multiword scans and uncertain mask padding

`runs/expansion-guidance-001/multiword-conditions.json` separates two conditions
on six new two-word locations from NASA pages 175, 190 and 217. Each condition
uses the same original scan pixels, supplied target locations and 300-entry
vocabulary across tools. The known-width condition supplies padding-corrected
ink width; the opaque condition supplies a total-padding interval of 0–12 pixels.
Actual padding is four pixels and is withheld from opaque adapter inputs.
These are six locations tested twice, not twelve independent examples.

| Configuration | Known width: correct unique first | Uncertain padding: correct unique first |
| --- | --- | --- |
| Local width/fringe | 0/6 | 0/6 |
| Local default BERT context | 2/6 | 2/6 |
| Local paragraph context, word normalization | 4/6 | 4/6 |
| Alex bundled fonts, wide tolerance | 0/6 | 0/6 |
| Alex matched catalog adaptation, wide tolerance | 0/6 | 0/6 |
| Morten matched catalog adaptation | 1/6 | 1/6 |

All 144 executions succeed. Local physical compatibility retains five of six
truths with known width and all six with uncertain padding, with mean surviving
sets of 20.17 and 23.83 candidates respectively. All three local modes have
identical complete physical reports; context only changes ordering. Width alone
ties the truth for first on three opaque cases, but never ranks it uniquely first.
No local mode rejects the vocabulary in any truth-absent case. This is a scoped
ranking improvement over the tested physical-only upstream configurations,
not evidence that arbitrary missing text can be uniquely recovered. Upstream
paid AI paths and full document-loading/detection workflows remain unmeasured.

Page screening, source extraction and manual review are retained under
`runs/multiword-preparation-001`. The six labels were corrected against full-size
pixels before scoring after reduced previews caused three transcription errors;
the failed first preparation is retained. The final overlay is
`runs/multiword-suite-002/reviewed-targets.png`. Page 217 is monospaced. The phrase
`additional resources` also has an unmasked occurrence, so this is an occurrence
holdout, not a lexical or document holdout. Distractors come from fixed other
pages of the same report and include one-, two- and three-word phrases.

Optional `expand --width-guidance` now provides fixed reference-string widths
using the solver's measurement function across calibrated font hypotheses.
Anchor contributions and padding intervals are preserved; pixelation evidence
does not receive a fictitious scalar width. Ollama's optional `--plain-gap`
consumes these measurements with compact passages and a text-list response.
On the nine reused NASA development omissions, both plain-gap and
plain-gap-plus-width propose zero exact truths, versus one with the earlier
attributed prompt. These options remain experimental; no measured prompt
improvement is claimed.

The fresh opaque-condition expansion test also completes for all six locations
with each Gemma configuration. Both the attributed prompt and plain-gap with
width guidance propose zero exact truths; none of their added candidates passes
the physical filter. Traces retain outputs that repeat surrounding context or
suggest grammatically unsuitable short text. Thus the closed-list ranking gain
does not extend to successful open generation on this cohort. The two prompts
and all runtime parameters were frozen before scoring these locations.

The measured binary and source snapshot are retained alongside `frozen.json`.
`final-integrity.json` verifies unchanged solver/provider/runtime hashes and
identical physical reports across all 24 local case inputs. The sole post-freeze
evaluation-script change selects a case-ID prefix, with its diff preserved.
All 91 Rust tests, 16 Python provider tests, the phrase-locator test, formatting
and all-target Clippy pass.

## Direct span completion and prompt diagnostics

`runs/ollama-diagnostics-001` retains 24 calls across two installed chat models,
three reused prompts and four formats: the compact JSON prompt with separate
instructions, merged instructions, a simple cloze request with a schema, and the
same cloze request without constrained output. Temperature is zero throughout.
The two real omitted strings are never proposed; the simple prompt improves the
format of answers to the easy thermometer control. Unconstrained outputs often
contain Markdown or prose that fails strict JSON parsing. Neither changing role
placement nor dropping the schema establishes a recovery gain.

The installed Gemma template renders separate system instructions as an extra
user turn. The provider now places instructions inside the initial user prompt
for known Gemma architectures, using model-family metadata, with explicit
`--system-placement=separate` to reproduce historical calls and `inline` for
custom templates. This follows
[Google's documented formatting](https://ai.google.dev/gemma/docs/core/prompt-structure).
It is a role-format correction, not an accuracy claim: one merged control adds
square brackets around its answer. Llama keeps its existing separate system role.

`scripts/span_proposal_provider.py` adds offline
[T5-base](https://huggingface.co/google-t5/t5-base) span-denoising proposals.
Its pinned safetensors checkpoint, configuration and tokenizer are verified
before and after loading. Both visible sides condition generation, with no
replacement length or candidate vocabulary supplied. The decoder starts with
the opening sentinel and must emit the closing sentinel; incomplete beams are
discarded. Default budgets are 32 beams, 24 generated tokens and 64 visible
tokens per side. Independent occurrence proposals are interleaved and checked
jointly by Rust. Scores remain diagnostic and do not alter physical filtering.

The first integrated run exposed a validation bug: Transformers pads finished
beam rows with repeated closing sentinels, while the validator expected PAD
tokens only. `runs/span-expansion-001` retains those errors and is incomplete.
After fixing that suffix check, without changing model or search settings,
`runs/span-expansion-002` freezes the corrected source and reruns all 31 reused
omissions. Completed spans, raw sequences, rejected-beam counts, model hashes and
both initial and expanded physical reports are retained. These are development
results, not a new holdout or a matched comparison against upstream AI features.

The 21 Python provider tests pass: 10 Ollama, 3 masked-context, 4 masked-proposal
and 4 span-proposal tests. Earlier totals of 16 or 15 refer to all three existing
provider modules, not just Ollama. Both model downloaders successfully reverify
the installed artifacts with their pinned hashes. Rust code is unchanged in this
iteration; its prior test results do not need a new build for the Python provider.

All 31 corrected expansion/ranking cases complete successfully:

| Reused omission cohort | Exact truth proposed and compatible | Correct unique first after expansion |
| --- | --- | --- |
| Multiword NASA, uncertain padding | 2/6 | 1/6 |
| NASA single words | 1/9 | 0/9 |
| Senate single words | 5/12 | 4/12 |
| EFTA single words | 0/4 | 0/4 |

The multiword truths are `partial control` (rank 2) and `probable cause` (rank 1),
where the previously tested Gemma configurations proposed neither. On the other
three cohorts, T5's recall is below the earlier BERT proposal configuration's
6/9, 8/12 and 1/4. Search budgets differ substantially: T5 returns at most 32
beam completions per occurrence, while BERT explores 256 singleton alternatives
plus longer beams and permits up to 512 proposals. This is not an equal-budget
model comparison. The common final ranker is still paragraph/word BERT, and
expanded vocabularies differ between models. T5 remains an optional provider.

`runs/span-expansion-002/final-integrity.json` verifies unchanged frozen runtime
and provider files, identical initial physical reports, unchanged source projects,
and identical expanded physical evidence before and after contextual ordering
for every case. No model download or inference used credentials. The separate
Ollama diagnostic server was stopped after its 24 calls completed.

## Sequence scoring and a common-language comparison

`context-rank --sequence-score` keeps the whole replacement's log sum rather than
dividing by token or word count. `scripts/span_context_provider.py` scores each
candidate token plus the closing span sentinel using the pinned T5-base model.
The encoder sees only visible context and one gap; it receives neither candidate
text nor length. A real, tiny T5 regression test verifies causal prefix scores,
the closing marker, aliases, batching and item-order invariance. Rust tests verify
that normalization changes ranking while preserving the entire physical report.

The paired development evaluation in `runs/span-ranking-001` fixes candidate
lists, paragraph context and physical evidence. Its primary comparison is T5
sequence sums against saved BERT paragraph/word ranking. The 31 closed lists,
31 T5-expanded lists and 25 BERT-expanded lists reuse locations and are not 87
independent examples. Runtime, model and search settings were frozen before
scoring; all 91 Rust tests, 24 Python provider tests, formatting and Clippy pass.

All 87 paired rankings complete successfully:

| Fixed candidate-list track | Saved BERT/word: correct unique first | T5/sequence: correct unique first |
| --- | --- | --- |
| Closed lists, truth present | 20/31 | 22/31 |
| T5-expanded lists | 5/31 | 6/31 |
| BERT-expanded lists | 4/25 | 6/25 |

Closed-list improvements are multiword 4/6→5/6 and Senate 7/12→9/12; NASA regresses
7/9→6/9 and EFTA remains 2/4. In T5-expanded lists, Senate improves 4/12→5/12;
other cohorts are unchanged. In BERT-expanded lists, NASA improves 0/9→1/9 and
Senate 4/12→5/12, while EFTA remains 0/4. Those expanded lists contain only 8/31
and 15/25 exact truths respectively, limiting what any ranker can recover. The
new scorer remains optional; aggregate gains do not establish universal gains.

An explicitly post-hoc arithmetic control recomputes token, word and sequence
normalizations from saved scores, without new inference or tuning. BERT sequence
sums score 21/31 on closed lists and 6/25 on BERT-expanded lists; the latter equals
T5 sequence scoring. T5 word means also score 22/31 on closed lists, but only 4/25
on BERT-expanded lists. Thus part of the improvement comes from normalization,
and cannot be attributed solely to changing models. Full results for all six
model/normalization combinations are in `normalization-diagnostic.json`. Defaults
were not selected or changed based on this post-hoc comparison.

A separate common-language control scores the entire closed vocabulary once
with T5, then applies each tool's saved compatible set. Every tool gets identical
language scores and visible context. External input hashes must match, and the
local scores/ranks agree with direct Rust/provider executions. This isolates
geometric filtering from language-model differences; it does **not** run Alex's
Claude feature or reproduce upstream OCR context selection.

| Cohort | T5 alone | Local geometry + T5 | Alex bundled/wide + same T5 | Alex matched catalog/wide + same T5 |
| --- | --- | --- | --- | --- |
| Multiword, opaque padding | 2/6 | 5/6 | 1/6 | 2/6 |
| NASA single words | 1/9 | 6/9 | 2/9 | 1/9 |
| Senate single words | 4/12 | 9/12 | 8/12 | 5/12 |
| EFTA single words | 0/4 | 2/4 | 2/4 | Not measured |

These are correct unique first ranks with truth supplied in the vocabulary.
Across the 31 cases covered by all three applicable columns, language alone
scores 7/31, local geometry plus T5 22/31, and Alex bundled/wide plus the same T5
13/31. The EFTA-only original narrow Alex filter scores 1/4. This supports a
scoped geometric advantage under a common language model, including a tie on
EFTA; it does not establish superiority over every upstream configuration.
Morten exposes no compatible set and is excluded from this control rather than
being assigned an invented cutoff. Its earlier direct comparisons remain separate.

Full candidate scores, model inputs, compatible-set sizes, truth retention and
hashes are retained under `runs/span-ranking-001/common-language`. All cases are
reused development data with supplied target locations and controlled masks;
public pretraining contamination remains unexcluded. Neither filtering nor a
first-place language score proves recovery of an unknown original redaction.

## Fresh linked occurrences on real scans

`runs/linked-scan-evaluation-001` tests six supplied same-text pairs at twelve
new locations in the previously used NASA NTRS 19860015255 document. Pages were
predeclared, screened by a prose rule, and words selected deterministically from
high-confidence OCR before scoring. The retained pages are 100, 160, 200 and 240;
the last is a small typewritten facsimile. All labels and mask bounds were
visually reviewed. Fixed crops retain both columns. Original scan images are
one-bit, 300 DPI; no color-subpixel evidence is assumed.

Each pair is evaluated first-only, second-only and jointly, with the true word
present or absent in the fixed 300-candidate list. These are 36 correlated cases,
not 36 independent examples. The words are management, provide, assurance,
flight, another and quality. Selected locations are masked on every input image;
unselected repeated words can remain visible. This is an occurrence holdout in
a previously used document, not a document or lexical holdout. Target locations
and correct links are supplied. Unknown total padding is 0–12 pixels, with
measurement sigma 1.5. The actual four-pixel mask padding is withheld.

Three local modes use the same frozen 37-font catalog: geometry only, paragraph
BERT sequence scoring and paragraph T5 sequence scoring. Two external modes use
our declared joint extensions of the pinned Alex and Morten per-occurrence width
scorers. Their input hashes must match local joint cases. These extensions are
not native upstream features, and their geometry-only ranking is distinct from
the language-assisted local modes. The benchmark does not run upstream Claude.

The small typewritten page fails local calibration and produces no usable Morten
profile. Those executions remain failures/unsupported in the six-pair totals.
A separate post-hoc diagnostic reproduces the local failure: all 74 font/kerning
fits exceed allowed tracking and ink-bias bounds. Fitting with tracking fixed at
zero avoids rejection on these visible words. The best validation fit has 2.08 px
validation RMSE (five words) and 4.14 px audit RMSE (seven words). This diagnostic
is not a runtime change, font identification, or revised benchmark result.
The frozen inputs/runtime and both the diagnostic rows and script are retained.

All 132 executions completed: 96 local successes, twelve local calibration
failures, 22 external successes and two external unsupported cases. Correct
unique first ranks, with failures retained in the denominator:

| Mode | First occurrence | Second occurrence | Linked pair |
| --- | --- | --- | --- |
| Local geometry only | 0/6 | 0/6 | 0/6 |
| Local geometry + BERT sequence | 5/6 | 3/6 | 5/6 |
| Local geometry + T5 sequence | 6/6 | 3/6 | 5/6 |
| Alex matched/wide joint extension, geometry only | Not run | Not run | 0/6 |
| Morten matched joint extension, geometry only | Not run | Not run | 0/6 |

The linked local modes and Morten each fail/are unsupported on `provide`; Alex
completes it. Among the five pairs supported locally, BERT improves from 4/5
first-only and 3/5 second-only to 5/5 linked. Specifically, `another` moves from
ranks 3 and 2 to rank 1. T5 is already 5/5 on the first views, so linking yields
no additional correct first ranks on the supported subset. The all-six result
retains the loss of a usable answer when linking to the unsupported page.

Compatible-set sizes for the five supported pairs are first/second/joint:
`flight` 25/56/25, `management` 29/87/29, `assurance` 107/102/102, `quality`
76/73/73, `another` 27/97/27. Thus the joint set equals the better single view
in every supported pair. Linking reduces the mean from 52.8 or 83 to 51.2, but
provides **no additional width pruning beyond the better individual page** in
this cohort. Context combination, rather than complementary geometric sets,
explains the observed BERT improvement.

All successful local conditions retain the truth geometrically. Joint Alex
retains 4/6 and joint Morten 0/5 successful truths, despite their smaller mean
compatible sets (15.83 and 8.8 versus local 51.2). These are declared joint
width adaptations and do not justify comparing local language accuracy with
upstream AI. No successful truth-absent condition produces an empty or singleton
compatible set. Ranking still cannot establish that the original text is present
in the supplied vocabulary.

The final audit verifies identical physical reports across all three local modes
for 32 successful conditions. For the five fully supported pairs, in both
vocabulary conditions, joint compatible sets equal intersections, physical costs
add exactly once, visible contexts remain unchanged, and joint language scores
equal the mean of the separate occurrence scores within floating-point tolerance.
Unavailable audits for the failed pair are explicit. All frozen runtime, suite,
font and masked-image hashes verify. `summary.json` retains the metrics and
per-pair audits, including failures. This is a small, controlled same-document
comparison with supplied links; it does not establish unknown-text recovery or
a benefit from automatically inferred links.

## Bounded calibration fixes the unsupported scan

The previous linked test exposed a fitting failure on page240. The old robust
regression estimated size, tracking and ink bias without constraints, then
rejected fits outside their physical limits. The updated fitter solves each
weighted ridge regression within those same limits: size 2–1000 pixels, absolute
tracking at most size/4, and absolute ink bias at most size/2. It returns an
already-feasible unconstrained optimum unchanged; otherwise, it enumerates the
small boundary-constrained quadratic problems and selects the feasible minimum.
It does not widen parameter limits or use hidden candidate labels for fitting.
Font/correction selection still uses validation words, with separate audit errors
in the uncertainty calculation. Existing stored profiles require recalibration.

Three analytic tests cover active faces, edges and vertices, exact preservation
of an unconstrained feasible fit, and held-out width prediction when fixed-pitch
parameters are non-identifiable. All 94 Rust tests, all-target Clippy, formatting
and the release build pass. An independent Pillow raster check fixes 16 conditions
across four fonts, two sizes and two added-spacing values, with blur and noise.
Known word labels and pixel-measured boxes isolate fitting, so this is not an OCR
benchmark. Both binaries succeed in all 16 conditions. Fifteen selected fits are
identical; one monospaced condition improves audit RMSE from 0.5406 to 0.4499 px.
Aggregate audit RMSE changes 0.4827→0.4768 px. The authoritative artifacts are
`runs/bounded-calibration-raster-002`; repeated words/conditions are correlated.

The paired real-scan reevaluation in `runs/bounded-calibration-linked-001` retains
the same six pairs, 300 candidates and truth-present/absent conditions. All 36
geometry runs and 24 joint-context runs now succeed. All previously successful
linked-suite physical reports are unchanged. The formerly failing `provide`
occurrence now has 67 compatible candidates; its other occurrence has 92, and
the joint set has 65. This is a small complementary geometric reduction beyond
either single page. All other pairs retain their previous sets.

| Joint result, six supplied pairs | Previous fitter | Bounded fitter |
| --- | --- | --- |
| Supported truth-present cases | 5/6 | 6/6 |
| Geometry-only correct unique first | 0/6 | 0/6 |
| Geometry + BERT sequence correct unique first | 5/6 | 6/6 |
| Geometry + T5 sequence correct unique first | 5/6 | 6/6 |
| Truth retained among compatible candidates | 5/6 | 6/6 |

The new result is a development reevaluation following diagnosis of this case,
not a fresh holdout. The recovered page's validation-selected fit has 1.44 px
validation RMSE on five words and 4.74 px audit RMSE on seven. Its selected font
hypotheses are not evidence of the original producer/typeface; tracking reaches
its bound and substantial uncertainty remains. All successful truth-absent
conditions still have multiple compatible candidates. First-place language
ranking cannot establish that the vocabulary contains the original text.

A separate regression check recalibrates all 31 earlier scan cases with both
binaries on identical cached OCR and the same font catalog. Saved full-vocabulary
T5 scores are replayed only after exact context and surface checks. The old
binary reproduces every archived physical report. The bounded binary preserves
29 physical reports exactly and changes two profiles on page217, while preserving
all 31 compatible sets and every truth rank. Correct first remains 22/31; all
31 supplied truths remain geometrically compatible. Thus the prior equal-language
control against Alex retains its scoped result. This check uses cached OCR and
model scores, rather than new OCR/model inference. Artifacts are under
`runs/bounded-calibration-controls-004`. Setup attempts rejected by schema/hash
checks are retained and labeled separately from completed evaluations.

The joint common-language control now gives every candidate the same two visible
contexts and the same pinned T5 mean sequence score. Applying each tool's saved
geometric compatible set yields:

| Joint filter with identical T5 scores | Correct unique first | Completed cases | Truth retained |
| --- | --- | --- | --- |
| No geometric filter | 4/6 | 6/6 | 6/6 |
| Bounded local geometry | 6/6 | 6/6 | 6/6 |
| Alex matched/wide joint extension | 4/6 | 6/6 | 4/6 |
| Morten matched joint extension | 0/6 | 5/6 | 0/6 |

The unsupported Morten pair remains in the denominator. Full-vocabulary scores
reproduce direct local joint rankings and scores within numerical tolerance;
external input hashes match exactly. Local geometry removes higher-language-score
incorrect candidates in `flight` and `management`; Alex excludes the correct
`flight` and `provide`. The restored `provide` already ranks first by language
alone, so its improvement measures support/retention, not an additional language
ordering gain. All candidate scores and hashes are saved under
`runs/bounded-calibration-linked-001/common-language-002`.

This comparison supports a scoped geometric advantage under identical language
scoring and supplied links. The external joint combination is our explicit
adaptation of pinned upstream width estimates, not their native feature, OCR
context selector or AI integration. These six pairs were reused to diagnose the
calibration change; 6/6 is not an independent unknown-text recovery estimate.
The earlier failed execution and initial common-language setup attempt remain
archived, as do all truth-absent and ambiguous results.

## Visible-corpus retrieval and large candidate sets

The Rust `harvest` command collects words and same-line phrases from visible OCR
in imported projects. It records source pixels, project hashes, OCR indices,
tokens and boxes for every retained origin. Known masks are unioned across
identical page copies before selecting one OCR copy; repeated copies cannot
inflate frequency. Hidden/native PDF text is not consulted. Confidence, phrase
length, candidate and origin limits are explicit. Frequency orders the exported
vocabulary but is not a physical or contextual confidence score.

The first evaluation pools only the masked sample projects from the earlier
31 scan cases and six linked pairs. All vocabularies are generated before
consulting labels for grading; the original truth-containing candidate lists are
not passed to retrieval or ranking. Visible unmasked repetitions are permitted.
NASA contributes ten unique pixel pages and 6,285 candidates, Senate four pages
and 1,456 candidates, and EFTA one page and 225 candidates. No list is truncated.
All 37 T5 sequence rankings complete.

| Cohort | Cases | Correct text available | Correct text compatible | Correct unique first |
| --- | --- | --- | --- | --- |
| NASA multiword | 6 | 1 | 1 | 1 |
| NASA single word | 9 | 1 | 1 | 1 |
| Senate | 12 | 0 | 0 | 0 |
| EFTA | 4 | 0 | 0 | 0 |
| NASA linked pairs | 6 | 6 | 6 | 5 |
| Total | 37 | 8 | 8 | 7 |

The origin audit checks all 10,730 retained source records against cached OCR,
project hashes and the union of known masks. Results and source snapshots are in
`runs/visible-corpus-evaluation-001`. This measures retrieval from visible
repetitions on reused controlled masks, not recovery of unseen names or a lexical
holdout. Its vocabulary budget differs from earlier model proposal experiments.

A second evaluation predeclares 21 additional NASA pages before extraction and
excludes every masked test page. All selected pages are retained. The combined
31 unique pages produce 15,753 candidates under a predeclared 50,000-candidate cap,
with no truncation. These are additional pages of the same source document,
not independent documents. The first expanded run retains six explicit failures
at the contextual ranker's former 2,048-compatible-candidate limit. That run's
availability counts cover successful rankings only and must not be interpreted
as full-corpus retrieval recall.

The ranker now supports bounded provider batches, retaining exact per-occurrence
accounting and a single total timeout. Default limits are 10,000 compatible
candidates, 2,048 items per call and 100,000 items overall; limits fail explicitly.
Every batch must return exactly its requested candidate/target pairs. Rankings
are emitted only after all batches succeed. Providers must return comparable
text likelihoods across calls, not scores normalized over each candidate batch.
Single-call traces retain their schema; multiple calls preserve every request,
response and hash under `provider.batches`.

All 100 Rust tests, formatting, all-target Clippy and the release build pass.
The batching tests cover 4,098 linked items, equivalence with one-call results,
cross-batch substitution, missing/duplicate scores and the shared timeout.
An exact cached-score replay preserves physics, context, scores and ranks on
all 31 earlier small-list cases. The expanded corpus was rerun with frozen source,
runtime, projects and model parameters; the earlier cap failures remain archived.

The completed batched run succeeds in all 21 NASA cases with the exact same
15,753-candidate file as the initial expanded run:

| Cohort | Cases | Correct text available | Correct text compatible | Correct unique first |
| --- | --- | --- | --- | --- |
| Multiword | 6 | 1 | 1 | 1 |
| Single word | 9 | 4 | 4 | 1 |
| Linked pairs | 6 | 6 | 6 | 4 |
| Total | 21 | 11 | 11 | 6 |

On these same 21 cases, the smaller corpus supplied 8 correct candidates and
ranked 7 first. Additional source pages therefore improve availability but reduce
first-place accuracy. The three newly available single words rank 85th, fourth
and fourth; one linked truth moves from first to second. Retrieval and contextual
disambiguation remain separate bottlenecks. No selection or score adjustment was
made after observing these results.

The final audit checks 54,343 candidate/occurrence requests, each exactly once,
across at most five provider calls per case. All target/text combinations are
unique. All 22,308 retained origins are verified against visible OCR and known
masks. The 15 cases that previously completed retain identical physical reports,
contexts, complete rank order and truth ranks; maximum sequence-score difference
is 0.00000721 from floating-point inference. All frozen source/runtime/input
hashes match. Artifacts, source snapshots, the 31-case regression replay and
completion audit are under `runs/visible-corpus-evaluation-003`.

This adds an attributed corpus proposal source and removes a measured capacity
failure. It does not demonstrate broad superiority over either upstream tool:
the masks and links are supplied, cases are reused, visible repeats are allowed,
and the candidate budgets differ from the earlier model-only proposal tests.

## Combining physical fit with contextual scores

The compatible-set filter can admit candidates whose physical fits differ
substantially. In a development error, T5 preferred `control and` over `assurance`
even though the former exceeded both linked width intervals by roughly 35 pixels.
Previously, contextual ordering discarded the physical cost after the hard
compatibility test. The explicit `context-rank --physical-weight` option now uses:

```text
combined_score = mean_sequence_log_score
                 - physical_weight * evidence_score / independent_observations
```

Positive weights require sequence scoring. Physical costs already include the
solver's font/model choices and uncertainty penalties; they are profile-fit costs,
not calibrated marginal likelihoods. Original physical reports, model scores and
observations remain available. Duplicate observations count once. Nonfinite or
negative weights, inconsistent occurrence counts and score overflow are rejected.
Weight zero is the default and omits the new optional fields, preserving previous
output. This is experimental ordering, not an identity probability.

A weight of 1 was fixed before selecting eight new mask locations. Two spans per
page were selected by a fixed hash order from high-confidence lowercase ASCII
OCR on NASA pages 55, 95, 135 and 195, with fixed length, margin and distinct-line
constraints. These pages had been imported for corpus harvesting but had not
served as masked test locations. All eight selections were visually checked and
retained, including the printed line-start fragment `velopment`; the other seven
are complete words. No labels, spellings, locations or weights were adjusted
after scoring. This is a new-location test in a reused document, not an unseen-
document evaluation. Two masks per page and same-source observations are correlated.

All selected spans are masked before fresh OCR/calibration and baseline execution.
Actual total mask padding is 4 pixels, while every tool receives an uncertain
0–12-pixel interval. The closed track supplies 300 candidates including the true
text. The open track harvests 12,814 candidates from 27 unique pages, excluding
all four original test pages entirely; correct text is never inserted into that
vocabulary. All 18,020 retained origins and the test-page exclusions are audited.

| New locations, eight spans | Correct text available | Correct text retained | Language ordering, correct first | Combined weight 1, correct first |
| --- | --- | --- | --- | --- |
| Supplied 300 candidates | 8/8 | 8/8 | 6/8 | 6/8 |
| Visible-corpus candidates | 5/8 | 5/8 | 2/8 | 3/8 |

The open-track improvement moves `below` from fourth to first; `showed` improves
from seventh to fourth. All other available truth ranks are unchanged. The corpus
does not supply `payload`, `disassembly` or the word fragment. The small gain does
not address missing candidates and does not establish a generally optimal weight.

All 24 pinned geometric baseline executions succeed. Without language ordering,
local geometry has zero unique correct first ranks but retains every truth in its
best tied group; Alex has one unique correct first rank and retains five truths
in its compatible set; Morten has zero unique correct first ranks. Giving the
closed vocabulary identical full-vocabulary T5 scores yields:

| Closed track with common T5 scores | Correct unique first |
| --- | --- |
| Language alone | 6/8 |
| Local compatible set, language ordering | 6/8 |
| Local compatible set, combined weight 1 | 6/8 |
| Alex matched-catalog/wide compatible set | 5/8 |
| Morten matched-catalog compatible set | Not available |

Alex's filter correctly promotes `disassembly`, where local ranking remains sixth,
but excludes `problem`, `should` and the word fragment. Local geometry therefore
retains more truths, while the equal-language first-place difference is only one
case. Morten's single-occurrence adapter reports rankings without a compatibility
threshold; no new cutoff or zero score is invented for the common-language table.
This comparison does not run upstream Claude or claim that our context selector
is their native AI integration. The matched font catalog and Alex tolerance 5.1
are explicit adaptations retained from earlier comparisons.

A separate exact-inference replay covers 31 earlier closed cases and 21 expanded-
corpus cases. Weight zero reproduces all 52 physical reports, contexts, scores and
ranks exactly. At weight 1 the closed cases remain 22/31 correct first. Expanded-
corpus results remain 6/21: the `assurance` gain is offset by demoting the correct
`additional resources` phrase. Some other ranks improve or worsen. No weight sweep
or change to the default followed these results.

All 101 Rust tests, formatting, all-target Clippy and the release build pass. The
final audit verifies 195 frozen files and snapshots, identical paired physics and
language scores, and 17,111 candidate/occurrence scores per weight mode counted
exactly once. The protocol-capture pass is explicitly ungraded; every scored
ranking uses the pinned T5 model and exact context/surface-checked replay.
Artifacts: `runs/combined-ranking-holdout-001` and
`runs/combined-ranking-replay-001`. These results support keeping a transparent,
optional combined scorer and continuing work on candidate discovery; broad
end-to-end superiority remains unproven.

## Corpus proposals and visible-text model adaptation

A fixed-budget follow-up tests the straightforward union of visible-corpus
candidates and T5 proposals on all eight locations from the combined-score
experiment. The proposal provider receives visible context but no vocabulary or
answer labels. Its budget remains 32 beams, 24 new tokens and 64 context tokens
per side. Existing scores are reused only after matching model identity, context,
candidate ID and text; added compatible candidates receive new scores. Both
ordering modes run through Rust with exact cached-score validation.

Base T5 adds 180 distinct candidates across eight cases, of which 28 pass geometry.
It supplies three correct spans already present in the corpus and none of the
three missing spans. Availability stays 5/8. Language first-place accuracy falls
from 2/8 to 1/8, and combined-score accuracy from 3/8 to 2/8. Every original
candidate retains its physical and language scores; new alternatives cause the
ranking losses. This experiment does not support automatically enabling that
union as an accuracy improvement. All results and pre-change source snapshots
are preserved in `runs/corpus-model-union-001`.

The next experiment introduces optional domain adaptation from visible OCR.
`export-training` uses the same visibility checks and source attribution as
harvesting. It unions masks across duplicate pixel pages and masks each selected
visible span while extracting its context. Labels retain their actual OCR
punctuation. Pixel-page hashes assign training/validation splits; exact duplicate
context/label triples are removed. This does not detect rescaled duplicates or
remove every same-document/template correlation. Original evaluation pages must
be excluded before export.

The predeclared NASA experiment excludes original pages 55, 95, 135 and 195 from
training, validation and retrieval. It exports 4,000 training examples on 17 pixel
pages and 512 validation examples on nine other pages. An independent audit checks
all source pixels, label/context OCR indices, masks, duplicate examples and page
exclusions. The first export failed its leakage check because a temporary label
was not masked during paragraph continuation; that error was fixed before any
training dataset was accepted. The failure remains archived.

The optional adapter changes 442,368 query/value parameters across 72 attention
projections in the pinned T5-base model. Base weights remain frozen. Rank 4,
alpha 8, learning rate 0.0003, batch size 4, 200 steps and seed 58431 were fixed
before training. Checkpoint selection uses only validation token loss at steps
0, 50, 100, 150 and 200. The selected step 200 lowers validation token NLL from
3.528039 to 3.207060. Training uses 800 example presentations from the exported
training pool; validation checks all 512 examples at each checkpoint. Lower
token loss is not evidence of improved redaction recovery.

Adapter files use safetensors and record hashes for the base model, weights and
dataset, plus exact projection names and training configuration. Both proposal
and context providers accept `--adapter`, validate the artifact and include its
identity in provenance. Base scores and adapted scores are separate: changing the
model requires rescoring. Tests verify zero-adapter equivalence, frozen base
parameters through optimizer steps, restoration, finite tensors and model/hash
mismatch rejection. The default provider path still uses the original pinned
model. Export tests cover label/context separation, masks, punctuation, page
splits, duplicate copies, deterministic order and caps. All 102 Rust tests and
nine span-provider/adapter Python tests pass, as do Clippy, formatting and the
release build.

All eight development cases complete with fixed proposal budgets and score
weights. Three controls separate proposal discovery from model ranking. No
adapter result is replaced by a base-model cached score. Correct-text availability
remains 5/8 in every condition:

| Candidates and ranking model | Language ordering, correct first | Combined weight 1, correct first |
| --- | --- | --- |
| Corpus, base model | 2/8 | 3/8 |
| Corpus + base proposals, base model | 1/8 | 2/8 |
| Corpus + adapted proposals, base model | 1/8 | 2/8 |
| Corpus, adapted model | 1/8 | 1/8 |
| Corpus + adapted proposals, adapted model | 0/8 | 1/8 |

The adapter does not generate `payload`, `disassembly` or the printed fragment.
It proposes one additional already-retrieved truth (`showed`), without improving
its final rank. Adapted likelihoods demote `below` and `should`. Thus this fixed
adapter improves its validation objective but worsens the redaction ranking task.
Neither adaptation nor unrestricted corpus/model union is promoted to a default.
The optional implementation and negative results remain available for reproducible
research; no claim of a neural accuracy gain is made.

The audit verifies separate base/adapted caches, identical physical evidence across
ranking modes, exact per-candidate score provenance, page exclusions, checkpoint
selection solely by validation loss, and frozen source/runtime/data artifacts.
The actual adapter-aware scorer executable also reproduces three in-process
likelihoods within 0.0001. Data export/audit, selected adapter, full histories,
three-control evaluation and snapshots are in `runs/domain-adapter-001`. These
are follow-ups on previously observed cases, not an independent new holdout or
an upstream-AI comparison.


## Broad development comparison and explicit padding-prior ablation

The new12-document fixture cohort separates four development documents from eight
test documents. All96 opaque development spans and96 absent controls were run
against the existing three configurations:576 successful executions, with source
pins and input integrity verified. Default Rust retains95/96 truths but ranks only
2/96 uniquely first; Alex matched-catalog/wide retains41/96 and ranks9/96 first;
Unredactron matched-catalog ranks6/96 first and exposes no compatible set. Median
surviving sets are40.5 for Rust and9 for Alex. Neither high retention nor these
closed-list results establish recovery or broad superiority.

An explicit uniform total-padding prior raises Rust unique-first counts to13/96
on the dictionary track and6/96 on natural distractors, with retention95/96.
The prior experiment reuses identical cached OCR/profiles; default-mode replay
reproduces all192 original native reports exactly. It executes768 mode/bank/control
combinations over96 original spans. The old best groups had fractional random-tie
credit10.01 and7.61 respectively. The natural result is therefore not a consistent
accuracy improvement, despite more unique first ranks. The new evidence type is
opt-in, preserves the old default, and does not turn ambiguity into verified text.
All105 Rust tests and Clippy pass, and four fresh integration cases exactly match
the cached prior outputs. See `docs/BENCHMARK_EXPANSION.md` and
`runs/padding-uniform-development-001` for assumptions, full paired results and
source snapshots. The independent test documents have not been scored.
