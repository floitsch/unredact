# Matched-input external comparisons

The Rust `compare` command evaluates adapter processes against the same inputs.
External adapters are now implemented and executed against pinned source. See
[BASELINE_SOURCE_REVIEW.md](BASELINE_SOURCE_REVIEW.md) for code findings,
adaptations, initial results, and limitations. The local single-observation
ablation is separately labeled; it is not an external baseline.

## Current external adapters

`scripts/prepare_external_comparison.py` generates contextual scan fixtures,
missing-name controls, source pins, and an adapter manifest for this implementation,
Alex's browser/WASM path at default/wide tolerances, and Unredactron's profiler
and original scoring loop. This is a single-occurrence track. The older exporter
below produces linked occurrences, which these external adapters explicitly
reject; it should not be used to claim a direct joint comparison.

With the workspace baseline tools installed:

```sh
python3 scripts/prepare_external_comparison.py --output runs/external-suite
target/release/unredact compare \
  --suite runs/external-suite/suite.json \
  --adapters runs/external-suite/adapters.json \
  --output runs/external-comparison --timeout-seconds 120
runs/baseline-venv/bin/python scripts/test_external_adapters.py
```

The generator defaults to eight names across three fonts, with one present and
one absent-vocabulary case per name/font. `--cases-per-font 2` produces the initial
smoke-test suite. Requests with several linked targets are unsupported by the
external adapters. The browser needs permission to create local sockets; in this
session that required execution outside the shell sandbox. All browser OCR assets
are local, and no model API is contacted.

Runtime/build dependencies used for the initial run:

- Python environment at `runs/baseline-venv`; exact package versions are captured
  in the suite's `python-environment.txt`. Baseline dependencies include Pillow,
  NumPy, OpenCV headless, pytesseract, pdf2image, and Playwright.
- Chrome at `/opt/google/chrome/chrome`; executable hash and runtime version are
  recorded. The generator currently assumes this Linux installation path.
- `runs/baseline-tools/browser-deps/node_modules`: npm packages `tesseract.js@5.1.1`,
  `tesseract.js-core@5.1.1`, `@tesseract.js-data/eng@1.0.0`, with a package lock.
- Original Alex crate built with `cargo build --locked --release --target
  wasm32-unknown-unknown`, output outside the checkout. Rust 1.93.0 was installed
  under `runs/baseline-tools` for this build. `wasm-bindgen` 0.2.114 generated the
  web package at `runs/baseline-tools/alex-pkg`; generated JS and WASM are hashed.

No tracked baseline files are patched. Generated browser modules, local asset URL
changes, and extracted function exports remain in each execution's work directory.
Pins/hashes cover declared artifacts, not the entire operating system or every
transitive runtime file. Record environment changes before comparing runs.

### Matched font catalogs

`scripts/adapters/alex_catalog.py` and `scripts/adapters/morten_catalog.py`
provide explicitly labelled catalog controls. Both verify each font's SHA-256
before copying its bytes. Alex's wrapper replaces the served font manifest and
assets; Unredactron's wrapper supplies a different font directory to the original
profiler. Fitting and candidate scoring remain upstream code. These configurations
are reported separately from the bundled-font configurations. Alex already
supports uploading fonts in its application.

The Unredactron wrapper preserves original TTF basenames because its profiler
sorts filenames, which can affect ties. Duplicate basenames are rejected. Alex's
wrapper uses safe, content-addressed browser aliases and records their mapping
to the original font files.

`scripts/summarize_comparisons.py` can combine completed reports with repeated
`--comparison PATH` arguments and a fresh `--output PATH`. It requires identical
suite hashes, per-case input hashes, and case/repetition coverage. Repeated
configuration results must agree exactly; they are consistency checks, not
additional trials. Runtime is retained separately for each source run.

The current contextual fixtures measure padding-corrected **ink bounds**.
Canvas `measureText().width` and Pillow `getlength()` measure **advance**, which
can differ because of side bearings and overhangs. Report this geometry
assumption with results. A separate advance/visible-anchor track is needed to
test other ways of constructing redaction boxes.

### Advance geometry and scan anchors

The contextual generator accepts `--geometry advance` to place right-hand
context at the candidate's natural Pillow text advance and supply that fractional
width to every adapter. `--geometry ink` retains the earlier fixture convention.
`--seed` selects a reproducible vocabulary sample and scan noise. Geometry and
seed are recorded in generator provenance. The advance mode draws no hidden
text into the supplied image; only its placement affects visible context.

The local adapter's `--scan-anchors` configuration replaces supplied mask-width
evidence with independently OCR-derived visible word endpoints. Its
`--mask-aware-ocr` option first blanks known/detected bars in an auxiliary OCR
input. OCR labels and calibration remain inferred from the image; ground-truth
fonts and hidden text are never passed to the adapter. `--learn-spaces` is a
separate calibration option. Report each configuration, including failures to
find reliable anchors; a failed execution is not an empty compatible set.

The adapter also accepts `--line-spaces` together with `--scan-anchors`. This
estimates a per-space correction from visible training-word pairs on each
target's row, independently for each retained font fit. It receives no candidate
identity during estimation. Report it as a separate configuration: the model
assumes equal expansion of ordinary spaces, retains existing profile uncertainty
and adds local residual scatter. Its report includes sample coordinates and
fallback reasons. A single matched input does not establish generalization to
other documents or justify interpreting the first-ranked text as recovered fact.

These are different measurement strategies on shared input images. The external
width scorers receive the declared advance width, while the anchor mode obtains
its measurement from visible pixels. An advantage on this track would not by
itself establish robustness to unknown mask padding, bad OCR, justification,
mixed styles, or arbitrary real documents.

### Controlled masks on real scans

`scripts/prepare_scan_holdout.py --review REVIEW.json --output NEW_DIRECTORY`
creates fixtures from reviewed source images, fixed body crops and visible word
labels. Image hashes are checked before preparation. OCR only locates the supplied
labels; it cannot choose alternatives. Duplicate labels require an explicit
occurrence index. Every selected region on a page is masked in every case.
An explicit reviewed `ocr_text` locator can retain the intended label when OCR
misreads it; provenance records the locator separately. `--vocabulary`,
`--candidate-count` and `--seed` permit a fixed alternative distractor pool,
including mixed grammatical roles instead of the default noun list.
Each page can specify its source `dpi` (default 400). Explicit reviewed
`trim_right` and `trim_bottom` pixel counts can exclude punctuation or adjacent
line ink accidentally included by OCR. These trims are recorded in provenance
and must be fixed by visual inspection before scoring. Review the final mask
overlays as well as the OCR transcription; a correct label alone does not
guarantee a correct measurement rectangle.
Unselected occurrences of the same word can remain visible: this is an
occurrence-level holdout, not a lexical-identity holdout. Duplicate counts are
recorded and should be reported with results. Original redactions stay intact
and unlabeled. The default vocabulary has 400 entries and a fixed distractor seed;
each present case has a matched absent-vocabulary case.

These fixtures provide padding-corrected ink widths. They are conditional on
readable reviewed words and selected page regions, not a random sample of all
redactions or a full-page layout benchmark. If a fixture exposes an implementation
bug and is used to fix it, label the subsequent run as development evidence;
another untouched set is needed to test generalization.

### Contextual ranking configuration

The local adapter accepts `--context-model MODEL_DIRECTORY` and
`--context-python PYTHON`. It runs the ordinary physical scorer, then the Rust
`context-rank` command with the offline BERT provider, and requires both physical
reports to agree exactly. Contextually ranked compatible candidates appear
first; incompatible candidates retain their physical ordering below them. The
compatible set is unchanged. `work/native-result.json` stores physical evidence
and `work/context-result.json` stores contextual scores, extracted context and
model/request/response provenance. AI-assisted and physical-only configurations
must be labeled separately: a contextual ranking gain is not a geometry gain.
Pretraining exposure to public documents is not excluded by holding out pages
from this project's development data.

The adapter forwards optional `--paragraph-context` and `--word-normalization`
flags to `context-rank`. Evaluate all four combinations separately when checking
these effects. The former changes only visible context extraction; the latter
changes only aggregation of the same model token log scores. Neither changes
the physical report or compatible set. Context diagnostics must include which
words/lines were used, including failed continuation attempts.

`scripts/evaluate_ocr_anchors.py` separately scores manually reviewed visible
neighbor transcriptions and confidence coverage. It records model hashes,
commands and raw TSV. It validates the TSV header because Tesseract can return
plain text with exit code zero when a requested config file is missing. The
local adapter can use `--ocr-tessdata DIRECTORY` with `--mask-aware-ocr`; manifests
must include `eng.traineddata` among hashed artifacts.

## Export and run a suite

Use a completed independent `scripts/evaluate_scans.py` run, or generate one:

```sh
python3 scripts/evaluate_scans.py --output runs/scans-for-comparison
python3 scripts/export_comparison_suite.py \
  --scan-evaluation runs/scans-for-comparison --output runs/comparison-suite

target/release/unredact compare \
  --suite runs/comparison-suite/suite.json \
  --adapters runs/comparison-suite/adapters.json \
  --output runs/comparison --repetitions 1 --timeout-seconds 120
```

The exporter produces present-name and missing-name cases, copied image files,
labels in `suite.json`, provenance hashes, and a manifest for local-joint and
local-single adapters. Use `--cases N` to select the first N source cases for a
smoke test; this is not a representative accuracy benchmark. Full comparisons
should use a predeclared suite and report every case.

The `candidate_ranking_from_images` track supplies raster pages, all redaction
locations, target boxes, padding assumptions, contexts, proposed cross-document
links, and the same candidate vocabulary. It does **not** supply font identities,
learned profiles, OCR transcriptions, or correct-answer labels to adapters. Each
adapter must perform its own OCR/font calibration. Native PDF-position analysis,
automatic mask detection, corpus-wide link discovery, and language-model proposal
quality require separate tracks; do not claim this track measures them.

The controller withholds labels from requests. It does not sandbox adapters
against other local files. External adapters must be reviewed for fidelity and
label leakage. Use only trusted code; the existing bounded subprocess transport
provides time/output limits and process-group cleanup, not filesystem isolation.

## Adapter manifest

```json
{
  "schema_version": 1,
  "required_external": ["alex-gilbert-unredact", "mortenfj-unredactron"],
  "adapters": [{
    "id": "alex-gilbert-unredact",
    "kind": "external",
    "executable": "/absolute/path/to/python3",
    "args": ["/absolute/path/to/reviewed-adapter.py"],
    "artifacts": ["/absolute/path/to/reviewed-adapter.py"],
    "repository": {
      "directory": "/absolute/path/to/checkout",
      "commit": "REPLACE_WITH_FULL_GIT_COMMIT_ID"
    }
  }]
}
```

The example requires real paths, an implemented adapter, and a full 40- or
64-character Git object ID. Branches and tags are rejected. `kind` is one of
`local`, `external`, `ablation`, or `test`. External adapters require repository
pins. The checkout must be clean, including untracked files; keep runtime output
under the provided work directory and adapters outside the baseline checkout.

Relative executable/artifact/repository paths resolve against the manifest's
location. Command arguments are passed literally without a shell; use absolute
paths within them. List adapter source, runtime binaries, relevant lockfiles,
and other executable dependencies in `artifacts`. The harness hashes declared
files and the executable before and after execution. It checks repository HEAD
and cleanliness both times. A source/artifact change invalidates results. Input
images are hash-checked before and after the run as well.

If source access is available in another terminal, check out the actual projects
into this workspace so an adapter can be written against their real interfaces:

```sh
git clone https://github.com/Alex-Gilbert/unredact.git baseline-alex
git clone https://github.com/mortenfj/unredactron.git baseline-morten
git -C baseline-alex rev-parse HEAD
git -C baseline-morten rev-parse HEAD
```

Inspect their licenses, build instructions, and code before adapting or running
them. Preserve the pinned implementation: wrapper-side changes to font fitting,
candidate scoring, or cross-document aggregation must be declared as adaptations,
not attributed to the upstream tool. If a baseline supports only one occurrence,
report that limitation or compare a single-occurrence track. Do not silently give
one tool extra evidence or tune thresholds using the evaluation labels.

## JSON protocol

An adapter reads one JSON object from stdin and writes one JSON object to stdout.
Diagnostic logs belong on stderr. Request shape:

```json
{
  "schema_version": 1,
  "input": {
    "schema_version": 1,
    "pages": [],
    "targets": [],
    "candidates": []
  },
  "work_dir": "/absolute/new/work/directory"
}
```

`pages` entries contain `id`, absolute `image` path, file `image_sha256`, `dpi`,
and `redaction_regions` rectangles. `targets` contain `id`, `page`, `bounds`,
`surface_kind`, `evidence`, `left_context`, `right_context`, and `dependence_key`.
Candidate entries use this project's `Candidate` JSON schema. All targets in a
case form the supplied shared-identity hypothesis; there are no precomputed
profiles or group labels. The image track cannot reconstruct native coordinates
from an OCR layer.

Success response:

```json
{
  "schema_version": 1,
  "status": "ok",
  "reason": null,
  "ranked_groups": [["candidate-a", "candidate-b"], ["candidate-c"]],
  "ranking_complete": true,
  "compatible": ["candidate-a", "candidate-b"]
}
```

Groups are ordered best to worst; members within a group are tied by evidence.
Preserve the upstream tool's actual ties; ordinal display order and AI tie-breaking
must not become a unique physical identification. IDs must come from the supplied
vocabulary and appear at most once. `ranking_complete: true` requires every input
candidate to appear. Partial rankings are allowed but omissions are counted.
`compatible: null` means the tool exposes no acceptance decision; an empty array
means it evaluated the vocabulary and accepted no candidates. Do not invent
compatibility thresholds for tools that only provide a ranking.

For an unsupported task or adapter error, return `status: "unsupported"` or
`"error"`, a nonempty `reason`, empty `ranked_groups`, `ranking_complete: false`,
and `compatible: null`. Missing compatibility output is not successful abstention.
Malformed output, timeouts, crashes, and unverifiable source versions are failures.

The included `scripts/adapters/local_unredact.py` implements this protocol for our
Rust CLI. It imports original images, removes OCR observations in supplied mask
regions, calibrates afresh, and invokes the solver. `--single-observation` is an
explicit local ablation. `--discover-fonts` enables the installed-font search;
keep configurations fixed before evaluating and record them in the manifest.

## Interpreting results

`comparison.json` records suite/manifest/input hashes, provenance, per-execution
responses and metrics, summaries, and `missing_verified_external`. The required
external IDs remain missing when only local or ablation adapters run. Each
execution directory contains the request, raw response when available, execution
record, and adapter work products. Post-run integrity failures invalidate execution
records; raw responses remain for inspection.

Metrics include unique correct first, truth in the best tie group, exact rank
interval, unranked candidates, compatibility recall when supplied, wrong unique
best, single wrong compatible candidate, and successful empty compatibility sets
when truth is absent. A wrong unique best is a ranking error, not necessarily a
claim of identity by the tool. Summaries retain successful/unsupported/failed
counts and separate truth-present/absent denominators. They do not turn unavailable
metrics into zero-error results.

Runtime is wall-clock time including adapter startup, OCR, calibration, and
scoring. Adapter order rotates by case/repetition; each invocation uses a fresh
work directory. OS/dependency caches still affect timing. The summary mean
includes failed/unsupported attempts; inspect execution status before comparing
runtime. Repetitions measure runtime variability, not additional independent
accuracy samples.

Repository pins establish source state, not that an adapter faithfully invokes
that source. Review adapters and inspect their output before reporting a direct
comparison. Passing harness tests, beating a local ablation, or winning a few
synthetic cases does not establish broad superiority over the external projects.


## Additional declared tracks

`scripts/prepare_joint_comparison.py` groups the contextual suite's three font
conditions into linked observations. Its external adapters use
`scripts/adapters/external_joint.py`, which combines original upstream widths.
The aggregation and its uncertainty assumptions are our extension, not native
upstream functionality. Keep this track separate from direct single-occurrence
results. Use a 360-second outer timeout for the three sequential observations.

`scripts/prepare_document_comparison.py` creates ordinary-word holdouts on an
existing raster document page. It masks four already-visible, reviewed words and
leaves all original redactions unlabeled. It preserves the contextual manifest's
fixed adapter configurations. Both generators expose their required input paths
through `--help`; use fresh output directories and retain generated provenance.

Latest completed artifacts: `external-context-comparison-003`,
`external-joint-comparison-001`, and `external-document-comparison-001` under
`runs/`. See BASELINE_SOURCE_REVIEW.md for results and limitations.
