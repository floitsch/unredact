# Diagnose and retry visible OCR

Context reports include extraction notes for each side of a target. For an
existing project, inspect detailed diagnostics without changing its OCR or
calibration:

```sh
target/release/unredact diagnose-context --project runs/my-document \
  --group hypothesis-1 --output runs/context-diagnostics.json
```

Use `--target occurrence-1` for one target; omit both filters for all targets.
The report identifies absent same-row text, low or nonfinite confidence, excessive
gaps, invalid word bounds and intervening masks. It records the contributing OCR
word indices, the word that stopped extraction and the gap in original pixels.
Reviewed supplied context is identified separately. These diagnostics describe
the conservative same-row extraction; paragraph continuation can add context
under its own recorded layout checks.

For new projects, retry visible OCR **before calibrating or registering targets**.
Save every known mask on that page in `masks.json`, using the original page-image
coordinates:

```json
[{"x": 100, "y": 250, "w": 160, "h": 30}]
```

```sh
target/release/unredact refresh-ocr --project runs/my-document \
  --page PAGE_ID --masks masks.json --retry-context-ocr --ocr-retry-regions 16
```

This starts with ordinary Tesseract PSM 3. Only masks missing visible context on
at least one side trigger PSM 6 and PSM 11 on horizontal strips. Each strip is
copied from an auxiliary raster with all known masks blanked. No resolution change
is applied. Words map back into original page coordinates, and their ink bounds
are measured on the original pixels. OCR words touching expanded dense mask
fringes are excluded. The logical mask boundaries stay unchanged so those
auxiliary fringes cannot block context at the target's own edge.

Retry selection uses visible context coverage only: preserve every previously
covered side, then require a strict improvement in covered sides or context word
count, capped at four words per side. Equal results retain the earlier OCR.
Candidate text, hidden labels and language scores never choose a retry. An OCR
confidence threshold and more available words do not establish transcription
accuracy; review the selected visible text.

The limit is 1–64 regions, with at most two retry calls per region. All attempts,
including errors and rejected alternatives, are saved in `ocr-inputs/*.json` with
crop geometry, masked pixel hashes, words, trigger diagnostics and selection
reasons. The record also includes baseline words, final diagnostics, original
logical masks (`input_masks`) and expanded OCR exclusions (`masks`). Retry records
include their region budget and policy version in their filename. `--tessdata`
retains the explicit English model hash in this record. Without
`--retry-context-ocr`, refresh keeps the ordinary PSM 3 path.

Refreshing a page with existing profiles or targets is refused because it would
invalidate OCR indices and calibration. To investigate an existing project's
diagnostic failures, import the original source at the same DPI into a **new
project directory**, supply all of the original logical masks, run the refresh,
then calibrate and register the targets again. Keep the prior analysis for
comparison. Legacy projects that already persisted expanded mask bounds remain
conservative; reimport with original logical masks to recover that distinction.

The exploratory two-page cached scan check lives in
`runs/ocr-recovery-review-001/summary.json`. It measures context availability,
not recovery of unknown hidden text, and does not modify frozen benchmark runs.

## Exploratory known-answer check

The current pipeline then ranked the first two case IDs in sorted order on each
of those two pages, fixing the four-case plan before calibration and model calls.
All four prior frozen runs had failed with `No local context available`. All four
current commands completed, and **two of four ranked the supplied correct text
uniquely first**. Every case retained its original 300-candidate vocabulary and
counts in this denominator:

| Public source, PDF page | Hidden text in controlled mask | First-ranked text | Correct |
| --- | --- | --- | --- |
| [Congressional Record, 1964, page 43](https://www.govinfo.gov/content/pkg/GPO-CRECB-1964-pt1/pdf/GPO-CRECB-1964-pt1-2.pdf#page=43) | and | and | yes |
| Same page | university | population | no |
| [Congressional Record, 1970, page 82](https://www.govinfo.gov/content/pkg/GPO-CRECB-1970-pt24/pdf/GPO-CRECB-1970-pt24-5-3.pdf#page=82) | the | the | yes |
| Same page | practices | services | no |

These were synthetic opaque masks over public text with known answers, rather
than previously unknown redactions. Fresh calibration used visible OCR outside
all eight masks on each page, the baseline font catalog, pinned local T5, and the
selected paragraph/sequence/physical-weight-1 settings. Controller truth was
separate from model inputs and assessed after all four scoring commands.

The result combines the logical-mask/fringe correction, optional OCR retries,
fresh calibration and current solver changes; it does **not** isolate an accuracy
gain from alternate PSM retries. The two-page availability check separately found
both-side context for 9/16 masks with fresh PSM 3 and 10/16 with retries; all 16
had at least one visible side in both conditions. This is an exploratory revisit
of known benchmark failures, not a new independent accuracy estimate. The sealed
plan, runtime snapshots, all four reports, and complete outcomes are preserved in
`runs/ocr-recovery-review-001/ranking-four/summary.json`.
