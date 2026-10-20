# Unredact

For a fresh GitHub checkout, start with [checkout setup](docs/CHECKOUT_SETUP.md).
The trained font model and compact benchmark reports are in
[`artifacts/`](artifacts/README.md); large experiment directories are excluded.

A Rust CLI and library for comparing candidate text against redaction evidence.
It learns typography from visible text, preserves native PDF coordinates, and
combines explicitly linked occurrences across pages and PDFs. Local language
models can propose candidates and rank them together with physical fit.

The selected pipeline now outperforms the pinned external baseline controls on
an independent test of **192 spans from eight documents**:

| Method | Correct first / 192 |
| --- | ---: |
| Rust geometry + local T5, physical weight 1 | **90** |
| Alex's matched filter + identical T5 scores | 46 |
| Unredactron's CSV hook + identical T5, scales 1 / 10 / 100 | 9 / 12 / 47 |
| T5 alone | 69 |

Every method receives the same supplied 300-candidate vocabulary. All 54 missing-
context failures remain in the denominator. Paired document and publisher-family
intervals favor the selected method. This compares declared baseline adaptations
with a common local model; Alex's paid Claude integration was not run. It does
not measure verified recovery of unknown real redactions.

Shared-cell pixelation scoring also improves over the previous scorer: 116/192
correct first at 8-pixel blocks and 87/192 at 16-pixel blocks, versus 43 and 0.
Learned height shows no consistent independent gain over unit height. The full
corpus contains 288 spans from 12 PDFs, half real scans, and six damage conditions.
The independent results cover opaque masks and both pixelation conditions.

Start with the [local analysis quickstart](docs/QUICKSTART.md). Read the
[completed results and limitations](docs/RESULTS.md),
[benchmark protocol](docs/BENCHMARK_EXPANSION.md), and
[pinned source comparison](docs/BASELINE_SOURCE_REVIEW.md).

## Build

See the [local analysis quickstart](docs/QUICKSTART.md) for import, mask setup,
the selected T5/physical ranking configuration, proposals and pixelation scoring.

The matched-input adapter harness is documented in
[docs/COMPARISON.md](docs/COMPARISON.md). It records source pins, ties, failures,
missing-name controls, and runtime. Reviewed adapters execute Alex's current
browser/WASM path and Unredactron's profiler/scoring loop. Their adaptations and
single-occurrence scope are documented explicitly.

Requires Rust 2024 edition support, a C compiler, `pkg-config`, and development
libraries for HarfBuzz, FreeType, Cairo, and Poppler GLib. Import also uses
Tesseract with English language data. Fontconfig discovers default fonts.
ImageMagick is required for input images other than PGM. Linux is the tested
platform. PDFs and single-frame images are supported.

```sh
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
```

Use `--offline` with Cargo if the dependency cache is already populated. Runtime
PDF/OCR processing is local. No model API is contacted automatically.

## Run the reproducible demonstration

Use a new output directory for each benchmark run:

```sh
target/release/unredact benchmark --output runs/demo
target/release/unredact inspect --project runs/demo
target/release/unredact solve --project runs/demo --group case-0 \
  --candidates runs/demo/candidates.json --output runs/demo/result.json
target/release/unredact solve --project runs/demo --group mosaic \
  --candidates runs/demo/candidates.json
```

The benchmark creates PGM pages, a native PDF, a project, candidate vocabulary,
ground truth, and metrics. Calibration words and target names are disjoint.
Opaque-mask tests use known padding and correct cross-document links.

An independent evaluator uses Pillow/FreeType, blur, noise, raster-only PDF
creation, Poppler rendering, and Tesseract OCR. The Rust app receives no truth
labels or word boxes from this evaluator. Requires Pillow and NumPy:

```sh
python3 scripts/evaluate_scans.py --output runs/scans
```

An explicit `opaque_uniform` evidence type is available for an optional uniform
prior over the declared total mask-padding interval. Existing `opaque` scoring
is unchanged. The prior breaks some width ties, but a broader development check
has not established consistent accuracy improvement after accounting for ties;
it is not enabled by default. Its assumptions and results are documented in
[runs/padding-uniform-development-001/README.md](runs/padding-uniform-development-001/README.md).

## Analyze documents

```sh
target/release/unredact import --project runs/my-project --dpi 200 first.pdf second.pdf
target/release/unredact inspect --project runs/my-project
target/release/unredact calibrate --project runs/my-project \
  --page PAGE_ID --id body --block BLOCK_ID
```

`inspect` lists page IDs, OCR typography blocks, proposed redaction rectangles,
and profiles. Add `--page PAGE_ID` to inspect the retained character coordinates
and full OCR observations. The page image is `pages/PAGE_ID.pgm` in the project.
All target rectangles and image measurements use **pixels at import resolution**.
Native character coordinates remain in **PDF points** (72 per inch).

On pages mixing body text and footnotes, propose size groups across OCR blocks:

```sh
target/release/unredact group-typography --project runs/my-project --page PAGE_ID
target/release/unredact calibrate --project runs/my-project --page PAGE_ID \
  --id body --size-group size-1 --learn-spaces
```

Choose an ID from that page's report; `size-1` is only an example. Grouping uses
training-word heights normalized by glyph shape in reference fonts. Validation
and audit pixels do not influence group assignment. Lines need three distinct
training words, and a group's largest/smallest line-height ratio is at most 1.10.
Original OCR line/block IDs remain intact, and the profile records group membership.
These are size hypotheses: same-size font families, bold, and italics may remain
mixed. Sparse lines remain unassigned. `--size-group` cannot combine with `--block`
or `--native`; target-to-group selection is currently explicit.

For scan-based width comparisons, add `--learn-spaces` to `calibrate` to learn an
average space adjustment from adjacent visible words. This is optional: it keeps
the word-calibrated font shortlist, fits on training pairs, and applies an
adjustment only when separate validation pairs improve by at least 5%. Pairs
containing audit word identities remain untouched until the final error check.
The profile records all used spans, rejected/accepted adjustments, and audit
errors. Phrase uncertainty includes pair prediction error and grows conservatively
with the number of spaces. Regions crossing detected or declared redactions are
excluded; add targets before recalibration when detection misses a mask.

This estimates local average spacing, not variable justification or a complete
scan distortion model. Prefer a consistent typography block. It currently applies
to width evidence; a nonzero adjustment is rejected for mosaic rendering, which
requires a profile without this option. Existing profiles load unchanged.

The Senate scan diagnostic extracts original 400-DPI page images with `pdfimages`
instead of downsampling the PDF. With the archived report downloaded locally:

```sh
python3 scripts/evaluate_senate_spacing.py \
  --source-pdf /path/to/report.pdf --output runs/senate-spacing
```

It measures visible OCR-labelled pair widths on six predeclared pages, with
per-page word-identity splits. It does not measure unknown-redaction recovery or
generalization to unseen pages. Whole-page and block results overlap and should
not be pooled as independent observations.

Omit `--block` only when visible text has uniform typography. Supply one or more
`--font /absolute/path/font.ttf` arguments to expand or restrict the font search.
Font face index 0 and default variable-font axes are currently used. Calibration
requires at least 12 distinct training words and four model-selection words. It tests
kerning on/off, estimates size/tracking/ink bias, and conditionally learns
regularized character/pair residuals. Word identities are assigned deterministically
to training (80%), model selection (10%), and audit (10%) partitions. Several
plausible font hypotheses are retained. Audit words never select a font, correction
model, or shortlist. When at least four distinct audit words are available, each
retained fit reports their RMSE/max error. Width uncertainty uses the larger of
selection and audit RMSE. Larger samples are preferable; four is only a minimum.
Old profiles remain readable and report the absence of independent audit data.
Size, tracking and ink-bias limits are enforced during fitting. This avoids
discarding an entire font hypothesis when an unconstrained fit compensates for
size with excessive letter spacing, as can happen with fixed-pitch text. The
limits are unchanged, and validation/audit errors still determine uncertainty;
a feasible fit does not identify the original font. Re-run `calibrate` on existing
projects to apply the updated fitter to their saved visible-word measurements.
New fits also record the font file's SHA-256. The engine loads an immutable copy
of those bytes and rejects scoring if the current file differs from calibration.
Legacy profiles without hashes still load and report that identity is unchecked.

To test whether source rasterization affected the spacing, optionally add:

```sh
target/release/unredact calibrate --project runs/my-project \
  --page PAGE_ID --id body --size-group size-0 \
  --hinted-ppem 12,16,24,48,96,192
```

This compares the default design metrics with FreeType normal hinted metrics.
`ppem` is the hypothesized source pixels per em, independent of scan DPI and the
fitted output scale. Supply at most 16 distinct values in 6–256. Every retained
fit records its `measurement_mode`; candidate and incremental width scoring use
that mode, including learned average space adjustments. Mosaic synthesis still
requires a design-metric profile. Installed-font screening remains design-based.
The option does not reproduce Word/GDI/DirectWrite or identify the source editor.

This expanded search is experimental: mode selection uses validation words, and
more hypotheses can overfit. On 115 matched Senate audit words, RMSE changed
from 2.786 to 2.958 pixels; on another 148 words it changed from 4.932 to 4.975.
Some pages improved, but aggregate errors increased, so it is **not enabled by
default**. Results and frozen settings are in `runs/senate-measurement-002` and
`runs/senate-measurement-holdout-002`. These use cached, uncorrected OCR and do not
establish hidden-text recovery. A separate four-word document ranking ablation
improved correct unique first place from 1/4 to 2/4, with all four truths still
compatible; neither configuration rejected the whole vocabulary when truth was
absent. See `runs/measurement-document-ablation-001`.
The larger synthetic scan ablation regresses correct unique first place from
4/24 to 2/24, with truth compatibility 24/24 for both configurations and no
whole-vocabulary rejection on 24 truth-absent cases. All 96 executions succeed;
results are in `runs/measurement-context-ablation-001`. The four-word gain does
not establish a general improvement or superiority over external projects.

For a document whose font is absent from the default five-family search, screen
the installed font library before full calibration:

```sh
target/release/unredact calibrate --project runs/my-project \
  --page PAGE_ID --id body --block BLOCK_ID --discover-fonts --font-shortlist 24
python3 scripts/evaluate_generalization.py --output runs/generalization
python3 scripts/evaluate_generalization.py --discover-fonts \
  --output runs/generalization-font-search
```

Fontconfig supplies font paths. Screening fits widths using up to 24 training
word identities and retains the requested 4–128 font files for full calibration.
Reports record the examined/unusable counts, training words, and shortlist scores.
Screening is approximate: a font omitted from the shortlist cannot be recovered
by later selection, and similar metrics do not establish the original font's
identity. Face index 0/default variation axes are used. Width calibration can
absorb horizontal stretch without learning the corresponding raster distortion;
this does not establish accurate pixel rendering for stretched or skewed scans.

Create a target JSON file:

```json
{
  "id": "occurrence-a",
  "page": "PAGE_ID",
  "bounds": {"x": 100, "y": 250, "w": 160, "h": 30},
  "profile": "body",
  "group": "entity-hypothesis-1",
  "surface_kind": "canonical",
  "evidence": {
    "kind": "opaque",
    "padding_min": 0,
    "padding_max": 12,
    "measurement_sigma": 1
  },
  "left_context": "The meeting included",
  "right_context": "and the other attendees.",
  "dependence_key": null
}
```

Padding is the **total left-plus-right padding**, expressed in pixels. It is an
assumption supplied by the analyst, not something inferred from a black bar.
`measurement_sigma` describes additional measurement uncertainty; calibration
error is added separately.

```sh
target/release/unredact target --project runs/my-project --spec target.json
target/release/unredact solve --project runs/my-project \
  --group entity-hypothesis-1 --candidates names.txt
```

Give related targets the same `group`. This explicitly asserts that their surface
forms belong to the same candidate entity; the app does not infer this from
matching widths. Use `surface_kind` to connect full names, surnames, initials, or
other supplied variants. Exact duplicate page/rectangle observations are counted
once. Set a common `dependence_key` for rescaled copies, copied passages, or other
observations that should contribute only once. Different groups are independent
analyses. Context-based link proposals can be examined separately as below.

## Proposing links across documents

```sh
target/release/unredact link-targets --project runs/my-project --output links.json
target/release/unredact solve-links --project runs/my-project \
  --anchor occurrence-a --candidates names.txt --max-links 20 --output scenarios.json
python3 scripts/evaluate_linking.py --output runs/linking-eval
```

`link-targets` compares existing targets on different pages, using supplied
left/right contexts or nearby OCR words on the same row. It excludes words
overlapping target masks or proposed redactions. At least three tokens are needed
on each side. Normalized ordered token matching tolerates limited spelling/OCR
errors; both sides must meet `--min-score` (default 0.8). This similarity is a
heuristic, not an identity probability. Different fonts, target widths, candidate
names, and existing group assignments do not influence link proposals.

The report preserves competing matches on the same destination page and hashes
the project state. Repeated templates may hide different names despite identical
contexts. No transitive merging or project modification occurs. `solve-links`
always includes the anchor alone, then evaluates each proposed pair separately.
For unknown dependence, it reports both assumed-independent and fully-dependent
results. Identical source pages, identical page images, and explicit shared
dependence keys contribute once. The scenario budget reports omitted links.
Errors in individual scenarios remain visible in the output.

These are conditional analyses: a compatible candidate does not verify a link.
The current implementation supports at most 2,000 existing targets, single-row
OCR fallback, and pairwise scenarios. It does not automatically create target
specifications, resolve general entity references, infer multi-hop links, or
estimate partial dependence between scans. Wider passages and multi-column
layouts may need supplied context.

## Native PDF positions

Black bars can merge with adjacent words in OCR. Before calibration, run
`refresh-ocr` with a JSON array of known mask rectangles (`x`, `y`, `w`, `h` in
page pixels):

```sh
target/release/unredact refresh-ocr --project runs/my-project \
  --page PAGE_ID --masks masks.json
```

This blanks detected and supplied bars in an auxiliary OCR input, then measures
recognized words on the original image. Original pixels remain unchanged;
auxiliary images, mask coordinates, pixel hashes and OCR output are saved under
`ocr-inputs/`. It refuses to replace OCR after that page has profiles or targets.

Scanned bars can have gray borders outside their detected black cores. The OCR
copy includes dense gray fringes contiguous with a near-solid bar, up to eight
pixels per side, and stops at white gaps. Input and effective mask bounds are
recorded separately. This helps avoid recognizing residual outlines as brackets;
the original image and its measured text bounds remain unchanged.

Automatic proposals are also checked against nearby text size. A small solid
shape under 40% of the median height of at least three nearby reliable OCR words
is excluded as a likely glyph fragment or rule. Without enough nearby words the
proposal remains. Raw and retained proposals are saved in
`pages/*-mask-proposals.json`. Explicitly supplied masks bypass this filter.

An optional English LSTM model can be supplied with
`refresh-ocr --tessdata /path/to/model-directory`. The directory needs
`eng.traineddata`; a separate TSV configuration file is unnecessary. Recognition
uses LSTM mode explicitly and records the model hash. The pinned official
`tessdata_best` model tested here is in `runs/ocr-models/tessdata-best`, with its
source commit, file hashes and Apache-2.0 license. The system OCR remains the
default. [Tesseract's model documentation](https://tesseract-ocr.github.io/tessdoc/Data-Files.html)
describes the speed/accuracy tradeoff; results on these scans are reported in the
source review.

For a scan with fully visible words immediately before and after a mask, add
`--scan-anchors` when registering a width/opaque target:

```sh
target/release/unredact target --project runs/my-project \
  --spec target.json --scan-anchors
```

This stores `scan_span` evidence containing the two OCR word indices and measures
the span from the left word's first ink pixel to the right word's last. Candidate
text is shaped together with both words. The mask selects the location; its width
does not enter the score. It assumes uniform typography and one ordinary space
on each side of the candidate. Optional `calibrate --learn-spaces` models a
profile-wide space adjustment.

For line-specific spacing, register with `--scan-anchors --line-spaces`. This
experimental option estimates an additional adjustment from at least three
distinct visible training-word pairs on the target's row. It excludes held-out
words, unreliable OCR and spans crossing detected or registered masks. The
median gap residual adjusts each space in the shaped candidate span. The report
records the pairs, adjustment and scatter separately for each font fit; existing
profile uncertainty is retained. Missing or implausible estimates fall back to
the profile's spacing with an explanation. Both complete scoring and incremental
LLM feedback use the estimate. This assumes equal expansion of spaces within a
line; it does not model tabs, mixed fonts or arbitrary justification.

Anchor selection fails on missing or unreliable neighbors, incompatible rows,
omitted recognized words, and additional detected masks within the span.
It does not silently fall back to mask-width scoring. Review the selected OCR
words in `project.json`; the confidence check cannot establish transcription
correctness. Automatic anchors use at least 1.5 pixels of measurement uncertainty.
Before registration, solving or LLM search, the app also checks the original
image hash and the gaps between anchors and mask. Four or more dark pixels in
either gap cause rejection for unexplained ink. This catches words omitted by
OCR, but noise can also cause rejection; it does not verify the anchor spelling.
An explicitly supplied `scan_span` specification may set its own positive
`measurement_sigma`. Register targets again after replacing/reordering OCR words.

On the earlier synthetic advance-width track, mask-aware OCR plus anchors and
learned spaces reached 3/16 unique correct first ranks, versus Alex with the same
font catalog at 2/16. Both put truth in their best tied group in 7/16 cases. Our
compatible sets averaged 129.5/400 candidates. Scanned-edge cleanup subsequently
improved OCR coverage on the four-word real-document check, but anchors still
identified none uniquely. On twelve further real-scan words, current width
scoring and Alex's bundled wide configuration each score 1/12 unique correct
first; our anchor modes without line spacing score 0/12. Adding `--line-spaces`
raises the real-scan result to 1/12 but lowers the earlier synthetic result from
3/16 to 1/16. It also widens compatible sets. The option remains experimental;
these results do not establish broad superiority.
[Full results and failed attempts](docs/BASELINE_SOURCE_REVIEW.md#target-row-spacing-experiment).

For an original text PDF, calibrate with `--native`. This learns advance widths
from Poppler's native character layout instead of OCR ink bounds. The app refuses
this mode for likely scans/OCR overlays. Native typography block names are
`FONT_NAME:SIZE` with size formatted to three decimals; omit `--block` only for
uniform text. Full native font names and sizes appear in `inspect --page`.

Use the following evidence object with a native profile:

```json
{
  "kind": "native_span",
  "left_anchor": 100,
  "right_anchor": 110,
  "prefix": "Contact ",
  "suffix": " ",
  "measurement_sigma_points": 0.01
}
```

The indices select surviving characters in the page's `native_chars` array.
`prefix` begins with the left anchor character and includes all known text before
the candidate. `suffix` contains known text after the candidate and must end in
whitespace before the right anchor. Both anchors must lie on the same horizontal
line and use the same font/size. The solver shapes the entire proposed span,
including its right boundary, and compares its advance to the native gap.
Raw PDF content-stream/TJ reconstruction and document-producer fingerprinting
remain future work; Poppler layout is the current native observation source.

## Pixelated text

For mosaic pixelation use an OCR/ink profile and this evidence object:

```json
{
  "kind": "mosaic", "block": 5, "phase_x": 0, "phase_y": 0,
  "origin_x": 8, "baseline_y": 34, "noise_sigma": 8
}
```

Grid phase, text origin, and baseline are relative to the target crop. Use
integer crop boundaries. The solver renders each candidate, tests small subpixel
alignment shifts, applies block averaging, and compares original grayscale
values. It excludes incomplete border cells: their original averages may include
pixels outside the crop. The crop must contain the entire concealed string;
candidate outlines extending beyond it are rejected to prevent matching just a
clipped prefix of an arbitrarily long phrase.

To infer integer square block size and phase from the image, use:

```json
{
  "kind": "mosaic_auto", "min_block": 2, "max_block": 32,
  "origin_x": 8, "baseline_y": 34, "noise_sigma": 8
}
```

Automatic inference looks for periodic discontinuities in both directions and
approximately constant pixels within cells. Several grid hypotheses may survive;
the result includes diagnostics and the selected grid/alignment per candidate.
It still needs the approximate text origin and baseline. No supported grid is an
analysis error, not evidence that every candidate is wrong. Analyze a crop before
solving, without supplying candidate text:

```sh
target/release/unredact analyze-pixels --image page.pgm \
  --rect 100 250 300 80 --min-block 2 --max-block 32 --output grid.json
python3 scripts/evaluate_pixelation.py --output runs/pixelation-eval
python3 scripts/evaluate_mosaic_solver.py --output runs/mosaic-solver-eval
```

For short crops, combine several disjoint regions on the same page. Put their
page coordinates in `regions.json`, as an array of `{ "x":100, "y":250,
"w":300, "h":28 }` objects. These regions must share a pixelation grid.

```sh
target/release/unredact analyze-pixel-regions --image page.pgm \
  --regions regions.json --output shared-grid.json
target/release/unredact rank-pixelation --project runs/document --profile body \
  --regions regions.json --target-index 0 --candidates names.json \
  --learn-vertical --output pixelation-ranking.json
```

`target-index` selects one entry in the region array. Use an OCR/ink profile
calibrated from visible text; run `refresh-ocr --masks regions.json` before
calibration. The optional height correction learns from visible training words
outside every supplied region and reports separate validation/audit errors.
The scorer searches text position within the mask and uses partial border blocks,
including crops with no complete blocks. Its process model assumes page-grid
averages copied inside masks while outside pixels stay unchanged. Cells touching
another mask are excluded. It reports all candidates, including unrenderable
ones, and does not infer that the correct text is present in the supplied list.

On96 development spans, shared-region/partial-cell scoring plus visible height
calibration gives69 correct-first rankings for8-pixel blocks and53 for16-pixel
blocks. The earlier single-crop scorer gives24 and1 respectively. These supplied-
answer development controls are not a general recovery rate. See
[benchmark details](docs/BENCHMARK_EXPANSION.md).

To combine occurrences believed to represent the same name or phrase, first run
`rank-pixelation` for each location using the same canonical candidate IDs. Alias
surfaces may differ between occurrences. Put the report paths in a manifest such
as `linked-occurrences.json`:

```json
{
  "group": "same-name hypothesis",
  "occurrences": [
    { "id": "document-a-page-4", "report": "a-ranking.json" },
    { "id": "document-b-page-9", "report": "b-ranking.json" }
  ]
}
```

```sh
target/release/unredact combine-pixelation \
  --occurrences linked-occurrences.json --output joint-ranking.json
```

The joint scorer shares one grid per page and one font choice per page/profile,
and charges the font penalty once. Same-page reports must use the same complete
mask list and calibration for each named profile. Identical page pixels,
dimensions and target locations count once. For rescaled or
recompressed copies of the same scan, give both occurrences the same optional
`dependence_key`; the first occurrence is retained. Different PDFs can contribute
independent evidence. Rankings remain conditional on the supplied identity link;
evaluate uncertain links as separate groups. Joint costs are model-fit scores,
not probabilities of an identity.

The independent Pillow/NumPy evaluations distinguish grid detection from
candidate ranking. Grid scores are not probabilities. Nearest-neighbor image
enlargement also produces blocks, and noninteger resizing can produce approximate
integer-grid matches. Strong blur and too few transitions can prevent detection.
Arbitrary blur and JPEG pipelines are not currently fitted. An opaque mask has
no recoverable interior pixel evidence; this path is specifically for retained
mosaic information.

## Optional neural font suggestions

`scripts/neural/` contains a trained CPU CNN for recognizing fonts from visible
text crops. It produces a ranked list of exact font files, with hashes and
per-crop similarities. The model, reproducible training data, frozen checkpoint,
and held-out evaluations are in `runs/neural-001`; see
[the neural instructions](scripts/neural/README.md).

```sh
runs/neural-venv/bin/python scripts/neural/infer.py \
  --input runs/neural-001/inference-request.json
```

Rust calibration can invoke the network directly:

```sh
target/release/unredact calibrate --project runs/my-project \
  --page PAGE_ID --id body --size-group size-0 \
  --neural-config runs/neural-001/calibration.json
```

The configuration names `python`, `script`, `model`, and `catalog` paths relative
to the configuration file, plus `top_k` (1–32) and `timeout_seconds` (1–3600).
The supplied configuration uses the trained model, isolated Python environment,
eight font suggestions, and a 120-second subprocess limit. The Python executable
retains its virtual-environment path. Model and catalog files are local; this
feature makes no model-service calls.

The engine assembles up to 24 images, each containing two or three distinct
training words from one line. It copies only pixels inside their OCR boxes,
excludes held-out words and supplied masks, checks for overlaps with other OCR
boxes, and inserts fresh white gaps. Audit/validation pixels and original
between-word pixels are not used. Duplicate training word identities are sampled
once. Images and hashes are saved under the project's `neural-inputs/`; the
profile records source boxes, model/script/catalog hashes, and returned scores.
New targets cannot overlap these font-selection samples without recalibration.

Suggested fonts supplement the original font set (or installed-font shortlist).
The existing physical calibration selects fits using validation words; audit
words remain withheld. Neural similarity scores do not affect candidate ranking.
Mixed styles within a selected group can still dilute pooled suggestions: this
is font proposal integration, not automatic font/style segmentation. Native-PDF
calibration does not accept this option. Manual `--font` arguments also remain
available. The network recognizes appearance, not size, kerning, or hidden text.

Actual Senate scan audits show no improvement over the default: on 115 matched
words, default/neural/full-catalog RMSE is 2.786/2.820/2.981 pixels; on another 148
words all three are 4.932 pixels. Searching the full catalog is a control for
simply adding fonts. These measurements use uncorrected OCR and correlated
within-page observations. Artifacts: `runs/senate-neural-calibration-001` and
`runs/senate-neural-calibration-holdout-001`. The option remains explicit and
does not replace the default search.

On four predeclared synthetic pages using bold/italic faces, including two
families absent from network training, the correct font appears in the eight
proposals for all four pages (ranks 1,1,1,2). Physical calibration selects those
exact fonts. Correct unique first place among 400 candidate names improves from
0/16 with default fonts to 3/16 with neural proposals; searching the full catalog
also gives 3/16. All retain truth compatibility in 16/16 present cases and
compatible alternatives in all 16 absent cases. All 96 executions succeed.
Mean runtimes are 1.51s default, 1.62s full catalog, and 4.60s neural. This supports
the benefit of font coverage, not a distinct neural accuracy or speed advantage.
Artifacts: `runs/neural-style-suite-002`, `runs/neural-style-comparison-001`.
Subsequent direct external runs on the identical styled inputs give Alex 1/16
unique correct first ranks with bundled fonts, and Unredactron 0/16. Supplying
both tools the same 37-file font catalog gives each 0/16; our full-catalog result
repeats exactly at 3/16. These are narrow synthetic ink-width tests: our compatible
sets still average 97.6 of 400 candidates. Alex already supports font uploads.
The neural model has not demonstrated an advantage over full-catalog search, and
the ink-versus-advance measurement distinction limits generalization. See the
[matched-catalog results](docs/BASELINE_SOURCE_REVIEW.md#styled-font-comparison-with-matched-catalog-controls).

The 93,016-parameter model gets 87.2% exact top-1 font retrieval for unseen font
families in the standard synthetic test, falling to 54.5% with stronger simulated
degradation. An independent Cairo renderer gives 81.9–82.6% for unseen families.
These are closed-catalog synthetic tests, not accuracy on real scans or redactions.

Renderer source checkouts are pinned under `runs/renderer-sources/`:
HarfBuzz 14.6.0 and FreeType 2.14.3. `scripts/inspect_renderers.py` records the
source commits, installed release versions, shared-library hashes, and font
hashes; the current inventory is `runs/renderer-inspection-003.json`. Installed
libraries were not rebuilt from these checkouts. Historical-library comparisons
remain future work. Width measurements support optional hinted metrics as
described above; Cairo image synthesis still has hinting disabled.

## Names and AI hooks

Build a candidate vocabulary from visible text in imported documents:

```sh
target/release/unredact harvest --project runs/my-project \
  --project runs/reference-documents --output runs/visible-candidates.json
target/release/unredact solve --project runs/my-project --group GROUP_ID \
  --candidates runs/visible-candidates.json
```

Each project can contain multiple PDFs. The output is also accepted by
`context-rank` and as the initial `--candidates` file for `expand`, so corpus
retrieval can be combined with local model proposals. Keep the harvest report:
its `origins` map resolves the source IDs retained in subsequent candidate files
to a project hash, page, pixel hash, word indices, original OCR tokens and boxes.

Defaults are OCR confidence at least 85, phrases of up to three words on one line,
10,000 candidates and eight retained origins per candidate. Change these with
`--min-confidence`, `--max-words`, `--max-candidates` and `--max-sources`.
Candidates are ordered by distinct pixel pages, then occurrence count, then text;
frequency is not an evidence score. The report counts omitted candidates/origins.
More than 100,000 distinct candidates requires a smaller corpus or phrase limit.

Harvesting verifies page pixels, excludes registered/detected masks, and rejects
empty or almost-solid OCR patches. Identical page copies share their mask
exclusions and contribute only one deterministic OCR copy. Phrases stop at masks,
unreliable words, sentence punctuation, large gaps and line boundaries. Case and
internal apostrophes/hyphens are preserved; edge punctuation is trimmed. Native
PDF text layers, spelling repair and line-hyphen repair are not used. Missing mask
detections and OCR errors remain possible; visible repetitions supply candidates
to test, not proof of the redacted text.

A `.txt` candidate file contains one complete candidate per line. JSON supports
multiple surface forms and provenance:

```json
{
  "candidates": [{
    "id": "example",
    "canonical_name": "Amber Vale",
    "surfaces": [
      {"text": "Amber Vale", "kind": "canonical"},
      {"text": "Vale", "kind": "surname"}
    ],
    "source_ids": ["user-vocabulary"],
    "context_score": null
  }]
}
```

The public-role starter vocabulary is in `data/public-name-seeds.json`. Each role
has a source and context scope; inclusion does not identify any redaction.
Candidate IDs must be unique across files. The true answer can be absent.

`--provider /path/to/executable` adds an optional source of names or AI-proposed
phrases. Repeat `--provider-arg VALUE` for arguments. The app launches the
executable without a shell, writes one JSON request to stdin, and expects one
candidate JSON object on stdout. Default timeout: 30 seconds; response limit:
8 MB. The request includes `schema_version`, `task: "suggest_candidates"`,
`group`, grouped target specifications (including both text contexts), and
`existing_candidates`. The adapter owns its model choice and credentials.

This one-shot provider proposes complete strings. AI scores only break
physical-score ties, and are not treated as probabilities.

### Local Llama or Gemma, without an API token

`scripts/ollama_provider.py` implements both provider protocols using a local
Ollama server and Python's standard library. It supplies both visible contexts,
all linked occurrences, and incremental geometry feedback. Model tags are
configurable; Llama 3.2 3B and Gemma 3 4B have been downloaded and actually tested.
Local inference needs no account or API token. Model weights use disk space and
inference uses this machine's resources.

Start the isolated server in a separate terminal:

```sh
sh scripts/run_local_ollama.sh
```

It binds to `127.0.0.1:11435`, disables Ollama cloud features, and stores models
under `runs/ollama-local/models`. If using a fresh workspace, download a model:

```sh
OLLAMA_HOST=127.0.0.1:11435 ollama pull gemma3:4b
```

Generate and physically score complete replacements:

```sh
target/release/unredact solve --project runs/my-document --group PERSON_A \
  --provider python3 --provider-arg scripts/ollama_provider.py \
  --provider-arg=--endpoint --provider-arg http://127.0.0.1:11435 \
  --provider-arg=--model --provider-arg gemma3:4b \
  --provider-arg=--trace --provider-arg runs/local-proposals.jsonl \
  --timeout-seconds 300 --output runs/local-result.json
```

Use the same provider arguments with `search` for iterative continuations; start
with `--rounds 4 --beam-width 4 --max-extensions 16`. Increase the adapter's
`--context-size` explicitly for larger groups/frontiers. It rejects prompts that
exceed a conservative byte budget instead of silently dropping context. The
adapter defaults to 16 proposals, 8 CPU threads, and a 240-second HTTP budget;
the engine's default 30-second subprocess budget should be increased on CPU.

Only loopback HTTP endpoints are accepted, proxies and redirects are disabled,
and cloud models are rejected. Traces contain complete document context,
generation options, raw outputs, and model/request/response hashes. They append
to the supplied JSONL file. Exact copies of both visible boundaries can be
removed from whole-sentence output; other malformed language can still pass
the JSON schema. Proposals are hypotheses, never established recovered text.
No model-generated confidence is used. The one-shot adapter supports one common
surface kind; mixed full-name/surname aliases need a variant-aware provider.

Actual smoke results are in `runs/ollama-evaluation-001` and
`runs/ollama-evaluation-gemma-001`: with up to 12 proposals, exact-word recall was
2/5 for Llama and 3/5 for Gemma on fresh hand-written cases. A sixth development
case is excluded. Both missed `confirmed` in a previously visible, deliberately
masked document word. Geometry rejected all Gemma proposals there; two incorrect
Llama proposals remained width-compatible. These tiny tests establish working
transport and expose quality limits; they do not establish recovery accuracy.
The Gemma run also exercises the actual Rust incremental-search command.

Gemma instruction placement uses the server's model-family metadata: instructions
are included in its initial user prompt, following
[Google's Gemma formatting guidance](https://ai.google.dev/gemma/docs/core/prompt-structure).
Forward `--system-placement=separate` to reproduce historical runs, or `inline`
for a custom template without a system role. The selected placement is recorded.
This fixes role formatting; it has not demonstrated an accuracy improvement.

Reproduce with `scripts/evaluate_ollama.py --help`. Provider validation tests:

```sh
python3 -m unittest discover -s scripts -p 'test_ollama_provider.py' -v
```

API details: [Ollama local authentication](https://docs.ollama.com/api/authentication)
and [structured outputs](https://docs.ollama.com/capabilities/structured-outputs).

## Contextual ranking of compatible candidates

`context-rank` adds a separate language-model ordering after physical scoring.
The ordinary `solve` result remains unchanged. Only physically compatible
candidates enter the contextual ranking; the result still reports physical
ambiguity and does not interpret a first rank as recovered text.

The included offline scorer uses the public
[BERT base uncased masked-language model](https://huggingface.co/google-bert/bert-base-uncased).
It masks every candidate WordPiece, then scores/reveals them from left to right
while retaining both visible contexts. Later candidate tokens are hidden when
an earlier token is scored. Scores are averaged per token and then across
independent target occurrences. This is a length-conditioned heuristic, not a
normalized probability of a replacement phrase. Capitalization is not modeled.

The installed `runs/neural-venv` environment includes Torch and Transformers.
For a fresh environment, install CPU PyTorch and `transformers==4.57.6` with
`safetensors==0.6.2`; the exact evaluated environment is recorded in
`runs/context-model-001/python-environment.txt`. Download the pinned model once:

```sh
python3 scripts/fetch_context_model.py \
  --output runs/context-model-001/bert-base-uncased

target/release/unredact context-rank --project runs/my-document --group PERSON_A \
  --candidates candidates.json \
  --provider runs/neural-venv/bin/python \
  --provider-arg scripts/masked_context_provider.py \
  --provider-arg=--model-dir \
  --provider-arg runs/context-model-001/bert-base-uncased \
  --output runs/context-result.json
```

Weights occupy about 440 MB. Downloading requires internet access; inference
runs offline without an account or API token. The fetcher pins the revision and
every file hash. The scorer verifies model/tokenizer hashes and uses safetensors;
reports include runtime versions, options, model hashes and exact provider
requests/responses.

If supplied context is empty, the Rust command extracts up to twelve nearby OCR
words on each side of the target row, retaining punctuation and word indices.
It stops at another mask, an unreliable OCR word or a large gap. It does not
infer reading order across lines or columns. Supplied context is separately
labeled. OCR errors, missing context and mixed languages can reduce ranking
quality. The scorer keeps at most 64 nearest WordPieces per side and reports
truncation; candidates need 1–32 known WordPieces. There is no confidence
threshold that can establish that the true text is in the candidate list.

The generic subprocess protocol has `schema_version: 1` and
`task: "score_context_candidates"`, a target-ID-to-context map, and `items`
containing candidate `id`, `target`, and the physically selected `text` surface.
Return `schema_version: 1` and `scores` with exactly one entry per item:
`id`, `target`, `token_log_score_sum` (finite and nonpositive), `token_count`
(1–128). Missing, duplicate and unsolicited entries fail the operation. This
hook can support other local models. Provider metadata is retained separately
from the engine's physical report.

Large vocabularies are scored in bounded provider calls. `--max-compatible`
defaults to 10,000 candidates; exceeding the configured limit fails rather than
silently dropping candidates. `--batch-items` defaults to 2,048 candidate/occurrence
pairs per call, with a total limit of 100,000 pairs. All calls share the configured
`--timeout-seconds` budget. Each response must cover exactly its own batch, and
linked-occurrence scores are combined only after every batch succeeds. Providers
must return comparable text likelihoods across calls, not scores normalized over
the current candidate batch. Single-call traces retain `request`/`response`;
multiple calls are recorded under `provider.batches`, with individual hashes.

To test a combined ordering, add `--sequence-score --physical-weight 1` to
`context-rank`. The score is the mean complete-replacement model log score minus
the selected weight times `physical.evidence_score / independent_observations`.
Both components and `combined_score` are reported for each compatible candidate.
Duplicate observations are still counted once; incompatible candidates remain
excluded. The default weight is zero, which preserves contextual-only ordering
and the existing report fields. Positive weights require sequence scoring.

This is an experimental score, not a posterior probability. Physical costs fit
uncertain fonts, surface forms and padding; their scale is not guaranteed to
match a language model. A larger weight can promote a worse textual answer when
the physical model is inaccurate. Use held-out examples to evaluate a weight
before relying on its ordering.

Context selection and score normalization are explicit options on `context-rank`:

- `--paragraph-context` can continue within the same OCR paragraph for up to two
  line transitions on each side, retaining the twelve-word budget. It checks
  line spacing, text height, column edges and unexplained gutters. Masks,
  unreliable words and unresolved line-end hyphens stop continuation. Word
  indices and `layout_notes` record accepted and rejected transitions. It makes
  no general reading-order guarantee for tables, arbitrary columns or bad OCR.
- `--word-normalization` divides each replacement's summed token log scores by
  its whitespace-delimited word count rather than its WordPiece count. This
  avoids rewarding a word merely because it uses more model tokens, but remains
  a length-conditioned ranking heuristic. Reports preserve `mean_token_log_score`
  and add `mean_word_log_score` plus an explicit normalization label. The selected
  surface text supplies the word count, including when aliases differ from the
  candidate's display name. Physical evidence remains unchanged.
- `--sequence-score` keeps the complete replacement's summed log score, averaged
  across independent occurrences, without dividing by token or word count. It
  conflicts with `--word-normalization`. Reports retain the token mean and add
  `mean_sequence_log_score`; physical evidence is unchanged.

The offline T5 scorer models variable-length replacements and their ending:

```sh
target/release/unredact context-rank --project runs/my-document --group PERSON_A \
  --candidates candidates.json --paragraph-context --sequence-score \
  --provider runs/neural-venv/bin/python \
  --provider-arg scripts/span_context_provider.py \
  --provider-arg=--model-dir=runs/span-model-001/t5-base \
  --output runs/span-context-result.json
```

Use `scripts/fetch_span_model.py` to obtain the pinned T5 weights as described
below. The scorer uses the same offline environment as the span proposal provider.
It scores every replacement token plus the closing sentinel, conditional on both
visible sides and the opening sentinel. Later replacement tokens cannot influence
earlier scores; the encoder is never given the candidate's length. Candidate
token counts include the closing sentinel. Actual token scores, IDs, input context,
truncation and model/source hashes are retained. The complete span score is a
model preference, not a probability that a hypothesis matches hidden text.

On twelve newly held-out Senate scan words with 300 candidates, the default
same-row/token configuration ranks
the correct word uniquely first in 7/12 cases, versus width-only 0/12, pinned
Alex configurations 0/12 and Unredactron with a matched font catalog 1/12. Language
alone scores 5/12; applying the physical filter improves that to 7/12. However,
the earlier four-word EFTA check regresses from width-only 1/4 to contextual 0/4.
The option therefore remains explicit. These are controlled masks over visible
words, not verified recovery of unknown original redactions.
[Methods, controls and limitations](docs/BASELINE_SOURCE_REVIEW.md#offline-contextual-ranking).

On a subsequent untouched NASA scan document (nine reviewed words, 300
candidates), paragraph context plus word normalization scores 7/9 correct unique
first, versus width-only 4/9, default contextual 4/9, paragraph/token 4/9,
same-row/word 5/9 and all three pinned upstream configurations 0/9. All 144
executions succeed. The local physical compatible sets remain identical across
options and retain all truths. However, removing the truth leaves one case with
a single wrong compatible candidate; none of the absent vocabularies is rejected.
These options remain explicit: development results are mixed, and known mask
padding, selected readable words and possible pretraining exposure limit the
conclusion. [NASA methods and full results](docs/BASELINE_SOURCE_REVIEW.md#paragraph-context-and-word-normalization).

## Expand the candidate list from visible context

`expand` asks a local model for additional words or phrases, checks them against
every independent linked occurrence, and writes a report that is also a candidate
file accepted by `solve` and `context-rank`. Existing identities, aliases and source
metadata stay intact. Exact spellings already represented by an alias are not
added as new identities. The proposal request contains visible context, without
the supplied candidate list or truth labels.

```sh
target/release/unredact expand --project runs/my-document --group PERSON_A \
  --candidates data/names.json --paragraph-context \
  --provider runs/neural-venv/bin/python \
  --provider-arg scripts/masked_proposal_provider.py \
  --provider-arg=--model-dir \
  --provider-arg runs/context-model-001/bert-base-uncased \
  --output runs/expanded-candidates.json
```

Omit `--candidates` to start from model proposals alone. The offline BERT provider
uses the same pinned model as contextual ranking: by default it considers 256
single-token alternatives and beams of 16 for two through six WordPieces, with
at most three whitespace words. It balances outputs across piece counts and
occurrences, then applies the caller's proposal cap (default 512). It supports
ASCII alphabetic tokens, including continuations; canonical tokenization does
not guarantee a valid word or grammatical phrase. `--case-variants` also proposes
titlecase and uppercase spellings because the model is uncased. Forward provider
options using `--provider-arg=...`.

The same command works with installed local Llama or Gemma weights. Replace the
provider arguments above with:

```sh
  --provider python3 \
  --provider-arg scripts/ollama_provider.py \
  --provider-arg=--endpoint=http://127.0.0.1:11435 \
  --provider-arg=--model=gemma3:4b
```

Start the workspace's local server with `sh scripts/run_local_ollama.sh` first.
No API token is required. Ollama defaults to sixteen proposals; its model digest,
options and request/response hashes are retained in the expansion report. Use
its `--trace` option for the full underlying model exchange.

`initial` and `expanded` hold full physical reports. `added_candidate_ids` and
`compatible_added_candidate_ids` distinguish new suggestions from geometric
survivors. `candidates` can be passed directly to `context-rank` for a separate
language-model ordering. The exact provider request/response and context word
indices are retained. Unknown targets, duplicate target/text proposals, ID
collisions, excessive output and malformed text fail the operation.

Expansion is bounded search, not an absence test. It can miss the true text,
introduce distracting alternatives, or worsen first-place ranking. A single
remaining candidate is still conditional on everything the search missed.
Mixed surface-kind groups need an alias-aware provider through `solve`; this
expansion interface tests a shared replacement spelling across the group.

Two additional experiments are explicit options. Forward `--plain-gap` to the
Ollama provider for a compact passage prompt with a text-list response. It uses
all linked contexts jointly; the first target ID is a stable routing ID and
`context_targets_used` records every occurrence. Add `--width-guidance` to
`expand` to include calibrated reference widths, which this prompt consumes.
The fixed examples are `a`, `word`, `example`, `information`, `short phrase` and
`several words together`; they are scale references, not suggested answers.
Ranges retain all calibrated font hypotheses. Anchored measurements include
the same visible anchors for every example; uncertain padding stays an interval.
Pixelated evidence does not supply a scalar width constraint. The actual solver
still checks each proposed replacement independently of these prompt hints.

Neither option is a recommended default yet. On the nine reused NASA cases,
compact prompting with or without width guidance proposes zero exact omitted
words, compared with one for the earlier Gemma prompt. These are development
results; adding geometric numbers to a prompt has not demonstrated a gain.

### Offline variable-length span completion

`scripts/span_proposal_provider.py` uses the pinned
[Apache-2.0 T5-base model](https://huggingface.co/google-t5/t5-base) to generate
the missing span directly, conditioned on both visible sides. It needs no server,
account or API token. Download its approximately 892 MB weights once:

```sh
python3 scripts/fetch_span_model.py --output runs/span-model-001/t5-base
```

You can also train an optional domain adapter using visible text from reference
projects. Keep evaluation pages out of these projects before exporting:

This remains experimental: the first eight-case development test lowered
validation token loss but found none of the missing spans. Using adapted proposals
and adapted ranking reduced combined-score accuracy from the corpus-only base
model's 3/8 to 1/8. The adapter is not enabled by default. Full controls and
artifacts are in `runs/domain-adapter-001`.

```sh
target/release/unredact export-training --project runs/reference-project \
  --output runs/domain-examples.json
runs/neural-venv/bin/python scripts/train_span_adapter.py \
  --dataset runs/domain-examples.json --output runs/my-domain-adapter
```

The exporter verifies source pixels, unions known masks across duplicate pages,
and creates span/context examples from reliable visible OCR. It preserves the
punctuation covered by each label and masks that span while collecting context.
Training and validation use separate pixel pages; exact duplicate examples are
removed. Rescaled or recompressed copies need to be removed from the reference
corpus separately. The default export caps are 4,000 training and 512 validation examples,
adjustable with `--max-train` and `--max-validation`. The trainer requires at least
32 usable examples in each split.

The default experiment trains 442,368 query/value adapter parameters for 200
steps with batches of four, keeping the base T5 weights frozen. It selects the
lowest validation token loss among the unchanged starting model and checkpoints
every 50 steps. The adapter is saved as safetensors with base-model, dataset,
projection and weight hashes. This is experimental adaptation to a document
domain; lower validation loss does not guarantee better redaction candidates.

Pass `--provider-arg=--adapter=runs/my-domain-adapter` to either the T5 proposal
or T5 context-score provider. Both verify the adapter and record its identity.
Adapter scores must not be mixed with cached scores from the base model; rescore
the candidate list when changing models. Without this argument, providers keep
using the original pinned T5 weights.

Then use the existing `expand` command with this provider:

```sh
target/release/unredact expand --project runs/my-document --group PERSON_A \
  --candidates data/candidates.json --paragraph-context \
  --provider runs/neural-venv/bin/python \
  --provider-arg scripts/span_proposal_provider.py \
  --provider-arg=--model-dir=runs/span-model-001/t5-base \
  --output runs/span-expanded-candidates.json
```

The Python environment needs Torch, Transformers and safetensors; the workspace's
`runs/neural-venv` has these dependencies. Inference is offline and verifies the
pinned artifact hashes before and after loading. It defaults to 32 beams, 24
generated tokens including the closing sentinel, and 64 visible tokens per side.
Forward `--beams`, `--tokens` and `--context-tokens` to adjust those bounded budgets.
Only completed spans become proposals. Raw token sequences, truncation flags,
model/source hashes and rejected-beam counts are retained in provider provenance.

Each linked occurrence proposes text independently; results are interleaved under
the shared cap and checked jointly by Rust. SentencePiece normalizes whitespace.
The model's beam scores are search diagnostics, not confidence, and do not change
the physical score. It can still miss the true text or add plausible distractors.

## Incremental next-word or token search

`search` lets a provider extend partial phrases over multiple rounds. Every
request includes both text contexts, measurements across the independent linked
targets, and feedback on completed candidates from previous rounds. Providers
can append a token, a word, or several words; spaces must be explicit.

Run the deterministic example provider against a generated demo vocabulary:

```sh
target/release/unredact search --project runs/demo --group case-0 \
  --provider python3 --provider-arg examples/word_provider.py \
  --provider-arg runs/demo/candidates.json --max-extensions 1024 \
  --trace runs/demo/search.jsonl --output runs/demo/search.json
```

The example is a dictionary adapter, not an AI model. An AI adapter implements
the same protocol. Version 2 requests have `task: "extend_prefixes"`, `round`,
`group`, `surface_kind`, `targets`, `prefixes`, `completed_feedback`, and `limits`.
Each prefix has an ID, exact text, optional contextual score, and measurements
for each typography hypothesis. A response looks like:

```json
{
  "extensions": [
    {"parent_id": "ID_FROM_REQUEST", "append": "Amber ", "complete": false},
    {"parent_id": "ID_FROM_REQUEST", "append": "another phrase", "complete": true,
     "context_score": 0.2}
  ],
  "exhaustive_for_prefixes": false
}
```

An empty `append` can mark a nonempty prefix complete. The provider must reference
only supplied prefix IDs and respect `limits.max_extensions`. Its exhaustiveness
field describes its own vocabulary; it does not establish complete coverage of
natural language. Search supports one common surface kind across the group;
use `solve` with candidate variants for full-name/surname entity matching.

Partial measurements treat the existing prefix as a complete string. They are
advisory: appending characters can change kerning and ligatures. Incomplete
prefixes are prioritized by overrun and contextual score, but are not rejected
as physical impossibilities. Complete phrases receive normal joint scoring,
including native-anchor or mosaic evidence where configured.

Bounds are explicit: `--rounds` (default 8), `--beam-width` (32),
`--max-extensions` per round (256), `--max-completed` (4096), and
`--max-text-bytes` (512). `--prefix` supplies known starting text. Dropped branches,
unfinished work, and the reasons for budget truncation appear in the result.
No complete proposals produces `result: null`, not a fabricated answer.
Both provider modes enforce the timeout through pipe closure and terminate the
invocation's process group, including descendants, on exit or failure.

Optional JSONL traces record requests, responses, hashes, round decisions, and
the final result. They contain document context and candidate text and must use
a new output filename. No credentials are needed by the engine; a model adapter
owns its credentials and network calls.

To check the extension machinery against full scoring of the same vocabulary:

```sh
python3 scripts/evaluate_search.py --project runs/demo \
  --candidates runs/demo/candidates.json --output runs/demo/search-evaluation.json
```

This comparison checks coverage, scores, compatibility, and evidence ties. It
does not evaluate a model's quality or compare with third-party projects.

## Reading results

Every result reports per-occurrence predicted widths or pixel errors, selected
font hypotheses, compatibility, and source IDs. Evidence ranks include a minimum
and maximum rank for ties. A tie broken by contextual score or deterministic
ordering remains a tie in the physical evidence.

`one_compatible_candidate` means one candidate in the supplied vocabulary fits
the current assumptions. It does not establish an identity. The 3-sigma filter
is a heuristic based on calibration residuals, not a calibrated posterior.
The selection error is distinct from the independent word audit reported in
`calibration_audits`. Audit words are visible vocabulary; their error distribution
does not establish coverage for names, mixed typography, or other document types.
Width costs include a log-scale term so a model cannot improve its score merely
by inflating its uncertainty. These profile-fit costs can be negative and are
neither posterior probabilities nor likelihoods marginalized over mask padding.

Important remaining work: held-out comparisons with third-party baselines;
broader independent and real-document benchmarks; learning scan distortion and mask processes;
noninteger pixelation grids and blur estimation; general cross-document alignment;
AI-provider quality benchmarks; and calibrated abstention under missing fonts,
missing candidates, and incorrect links. See `WORK_LOG.md` for current evidence.
