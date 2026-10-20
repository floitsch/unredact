# Unredact

**Compare candidate text against the evidence left by a redaction.**

Unredact is a local Rust CLI and library for analyzing redacted PDFs, scans, and
pixelated text. It learns typography from visible words, tests candidate widths
or rendered pixels, and can combine physical evidence with a local language model.
A browser viewer helps you mark targets and inspect their visible context.

**Current evidence:** the archived geometry + T5 pipeline ranked the correct text
first in **90/192 controlled opaque-mask cases**, compared with **69/192 for T5
alone**. We have **no verified recovery of a genuine redaction** whose answer was
unknown at analysis time. These are candidate-ranking experiments, with the
correct answer supplied among 300 candidates.

[Quickstart](docs/QUICKSTART.md) · [Fresh checkout](docs/CHECKOUT_SETUP.md) ·
[Detailed usage](docs/USAGE.md) · [Results](docs/RESULTS.md) ·
[Comparison protocol](docs/COMPARISON.md)

## What it can do

| Capability | What you get |
| --- | --- |
| **Analyze PDFs and scans** | Local import, page images, OCR, proposed masks, and retained native PDF character coordinates where available. |
| **Learn local typography** | Font, size, tracking, and width-error estimates from visible text, with separate model-selection and audit words. Several plausible font fits are retained. |
| **Test opaque-mask candidates** | Width compatibility under explicit mask-padding assumptions, or measurements from visible neighboring words. Reports preserve ties and competing explanations. |
| **Rank with visible context** | Optional local T5 scoring combined with physical fit. A named preset makes the selected configuration explicit; persistent workers reuse the model across batches. |
| **Analyze pixelated text** | Candidate rendering against shared pixelation grids, including partially visible cells and position search. |
| **Combine linked occurrences** | Joint evidence across pages or PDFs for an explicitly supplied same-entity hypothesis, with aliases and duplicate/dependence handling. |
| **Review targets visually** | A local browser viewer for drawing, moving, and resizing rectangles, previewing context, and exporting validated target JSON. |
| **Diagnose missing context** | OCR stopping reasons, mask exclusions, and optional regional OCR retries with recorded provenance. |
| **Make an explicit abstention decision** | A separate `assess` command applies a supplied policy to a saved ranking and records why it answers or declines. |
| **Audit the analysis** | JSON reports retain measurements, font hypotheses, model/provider provenance, and input hashes. |

Candidate proposals from local language models and visible reference documents
are also available. Their open-vocabulary recovery quality is not established by
the closed-vocabulary comparisons below.

## What it cannot establish

- **Text erased by an opaque bar is not available as pixels.** The method tests
  hypotheses using remaining layout and context. A plausible match is not proof
  of the original wording.
- **First place does not mean the answer was in the list.** In the archived
  geometry-only absent-answer controls, the solver rejected no entire vocabulary.
  Compatibility and ranking scores are not calibrated identity probabilities.
- **It cannot currently reconstruct whole redacted paragraphs or pages.** The
  evaluated workflow targets short text spans on one row. Links between occurrences
  are explicit analyst assumptions; automatic general entity resolution is absent.
- **Unknown typography, padding, and scan distortion remain material limits.**
  Missing fonts, mixed styles, handwriting, arbitrary mask padding, poor OCR, and
  strongly resampled pixelation need further validation.
- **A language model can prefer convincing but wrong text.** Public source
  documents may also have appeared in model pretraining. Holding documents out of
  our development set does not rule that out.
- **The latest engine has not had a full benchmark rerun.** The large comparison
  below predates recent dependence-grouping, compatibility, and OCR fixes. Those
  changes have regression tests and smaller exploratory checks.

## Measured comparisons

### Opaque masks: geometry and language together

The archived evaluation uses **192 spans from eight held-out public documents**,
with four other documents used for development. Half the test spans come from
scans. Every method gets the same 300-candidate vocabulary containing the answer;
language controls use identical cached T5 sequence scores. All **54 missing-context
failures** remain in the denominator.

| Method | Correct first / 192 | Rate |
| --- | ---: | ---: |
| **Unredact geometry + T5, physical weight 1** | **90** | **46.9%** |
| Unredact filter + T5, physical weight 0 | 86 | 44.8% |
| T5 alone | 69 | 35.9% |
| Alex-Gilbert/unredact filter + identical T5 | 46 | 24.0% |
| Unredactron CSV hook + identical T5, scale 1 | 9 | 4.7% |
| Unredactron CSV hook + identical T5, scale 10 | 12 | 6.3% |
| Unredactron CSV hook + identical T5, scale 100 | 47 | 24.5% |

These compare **pinned, documented adaptations** of the external projects.
Alex's optional paid Claude integration was not run. Unredactron's CSV confidence
scales are declared controls; scale 100 is a stress control. Unredactron receives
credit for its native selected match, including its tie choice; Unredact and Alex
require a unique first place. See the [source and adapter review](docs/BASELINE_SOURCE_REVIEW.md)
and [full results with uncertainty intervals](docs/RESULTS.md).

**Width-only ranking was weaker:** Unredact uniquely ranked 2/192 answers first,
versus Alex's 5/192 and Unredactron's 9/192. Unredact retained 188/192 correct
candidates as compatible, but the median compatible set contained 66.5 candidates.
Its measured advantage is in the combined pipeline; retaining many possibilities
is not recovery.

### Pixelation: comparison with our previous scorer

This uses the same 192 test spans with synthetic pixelation. It compares versions
of this repository's scorer, not external pixelation tools.

| Block size | Previous scorer | Shared cells, unit height | Shared cells, learned height |
| --- | ---: | ---: | ---: |
| 8 pixels | 43/192 | 115/192 | **116/192** |
| 16 pixels | 0/192 | **88/192** | 87/192 |

Failures remain in the denominator. Shared-grid handling improves these results;
learned height adds no consistent independent gain. See [pixelation results](docs/RESULTS.md#pixelation).

### Abstention: fewer answers, stronger selection

A separate retrospective experiment fitted a score-and-gap policy on four
development documents, then froze it before evaluation on eight other documents.
It uses archived rankings and includes both answer-present and answer-absent cases.

| Evaluation policy | Answered / 384 attempts | Correct / answered | Coverage |
| --- | ---: | ---: | ---: |
| Accept every usable unique top result | 275 | 90/275 | 71.6% |
| Frozen abstention policy | 43 | 43/43 | 11.2% |

The frozen policy accepted no wrong answer in 192 absent-answer controls. Its
observed 43/43 does **not** establish future 100% accuracy: cases share documents
and masks, and missing-font and incorrect-link robustness were not measured.
The policy is tied to the archived engine/model provenance and cannot simply be
applied to new-engine reports. [Method and limitations](docs/ABSTENTION.md).

## What happened on public documents?

We distinguish controlled masks, replays, and genuine redactions:

| Documents | Experiment | Outcome |
| --- | --- | --- |
| Congressional Record, 1964 PDF p. 43 and 1970 PDF p. 82 | Fresh OCR and ranking on four preselected artificial masks | **2/4 correct:** “and” and “the.” Previously all four failed for missing context. “university” and “practices” were still ranked incorrectly. |
| NIST AI Risk Management Framework; NASA Apollo descent propulsion report; USGS Weaverville and water-use reports; Census poverty report | Replay of 16 previously successful artificial-mask cases | **16/16 reproduced.** These were selected successes, not new recoveries or an accuracy estimate. |
| CIA Reorganization, NARA record 176-10033-10145: 2022 redacted and 2025 released versions | Inspection of genuine redactions with later-released ground truth | **Not scored.** The inspected omissions cover paragraphs or a page, beyond the current single-row method. |

The Congressional Record check used supplied 300-candidate lists and combined
OCR fixes, optional retries, fresh calibration, and the updated solver. It does
not isolate the effect of any one change. [Exact outcomes and source PDFs](docs/OCR_RECOVERY.md#exploratory-known-answer-check).

The CIA pair is available from NARA: [2022 redacted PDF](https://www.archives.gov/files/research/jfk/releases/2022/176-10033-10145.pdf)
and [2025 released PDF](https://www.archives.gov/files/research/jfk/releases/2025/0318/176-10033-10145.pdf).
We have not yet demonstrated recovery on a genuine redaction validated against a
later release. Testing short spans in such document pairs is a next evaluation step.

## How an analysis works

1. **Import** the document and identify the target regions.
2. **Calibrate** typography using visible words outside all known masks.
3. **Supply candidates** and state assumptions about padding and linked occurrences.
4. **Rank** by physical fit, optionally adding visible-context language scores.
5. **Review** the alternatives, diagnostics, and provenance; optionally apply a
   separately developed abstention policy.

For opaque masks, the surviving evidence is primarily geometry and context.
Pixelation retains additional image information that can be compared with rendered
candidate text. The same first-place label has different evidential support in
these two settings.

## Build

Linux is the tested platform. Install a Rust toolchain with 2024 edition support,
a C compiler, `pkg-config`, and development libraries for **HarfBuzz, FreeType,
Cairo, and Poppler GLib**. Document import also uses **Tesseract with English
language data**; Fontconfig discovers installed fonts. ImageMagick is needed for
input images other than PGM.

```sh
cargo build --locked --release
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The core CLI needs no Python or model API credentials. Optional local language
models require a Python environment and a separate checkpoint download. After
setup, PDF/OCR processing and local model inference run locally. Follow
[fresh-checkout setup](docs/CHECKOUT_SETUP.md) for dependencies and model paths.

## Try it

Generate a self-contained demonstration in a new directory:

```sh
target/release/unredact benchmark --output runs/demo
target/release/unredact inspect --project runs/demo
target/release/unredact solve --project runs/demo --group case-0 \
  --candidates runs/demo/candidates.json --output runs/demo/result.json
```

Open the local viewer:

```sh
target/release/unredact review --project runs/demo
```

Open the printed loopback URL. The viewer exports validated target JSON; register
it with the CLI to update the project. See the [viewer guide](docs/LOCAL_REVIEW.md).

For your own PDF, follow the [analysis quickstart](docs/QUICKSTART.md). Its explicit
`paragraph-sequence-physical-v1` preset selects paragraph context, sequence scoring,
and physical weight 1. The default CLI physical weight remains zero, so selecting
the preset matters when reproducing that configuration.

## Documentation and reproducibility

| Guide | Contents |
| --- | --- |
| [Fresh checkout](docs/CHECKOUT_SETUP.md) | Native dependencies, optional model setup, font checkpoint, baseline source pins. |
| [Analysis quickstart](docs/QUICKSTART.md) | Import, calibrate, register targets, rank candidates, and inspect pixelation. |
| [Detailed usage](docs/USAGE.md) | Full CLI workflows, native coordinates, linking, providers, proposals, and experimental options. |
| [Local viewer](docs/LOCAL_REVIEW.md) | Rectangle editing, context previews, and validated export. |
| [OCR recovery](docs/OCR_RECOVERY.md) | Diagnostics, regional retries, provenance, and the four-case scan check. |
| [Abstention](docs/ABSTENTION.md) | Policies, development/evaluation separation, integrity checks, and limitations. |
| [Results](docs/RESULTS.md) | Archived benchmark outcomes and uncertainty estimates. |
| [Comparison protocol](docs/COMPARISON.md) | Matched inputs, adapters, failures, ties, and reproducible commands. |
| [Benchmark design](docs/BENCHMARK_EXPANSION.md) | Corpus preparation, splits, damage conditions, and experimental history. |
| [Published artifacts](artifacts/README.md) | Trained optional font model, compact reports, and hashes. |

The broader corpus contains 288 spans from 12 public PDFs and six damage
conditions. The independent comparisons above cover opaque masks and two
pixelation conditions. Variants of a span are not independent samples.

Large `runs/` directories, source PDFs, third-party model weights, environments,
and baseline checkouts are excluded from Git. Published summary artifacts retain
original paths and hashes; they do not contain every input required to replay the
historical experiments. The preparation and evaluation scripts are included.
