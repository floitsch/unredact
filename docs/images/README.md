# README examples

These images illustrate actual app behavior and saved measurements. They are
not generated depictions of the interface or recovered document text.

| Image | Source |
| --- | --- |
| `local-review.png` | Screenshot of the running Rust local viewer, with paragraph context enabled and calibration overlays hidden. Zoom and scroll were adjusted using ordinary controls. |
| `candidate-ranking.png` | Two actual masked scan crops and top-three score excerpts from the four-case OCR experiment. The panels show one correct ranking and one wrong ranking; the overall experiment scored 2/4. |
| `pixelation.png` | Actual pixels and physical costs from a fresh built-in synthetic benchmark and `solve --group mosaic` invocation. |

The opaque-mask source is [Congressional Record, 1964, PDF page 43](https://www.govinfo.gov/content/pkg/GPO-CRECB-1964-pt1/pdf/GPO-CRECB-1964-pt1-2.pdf#page=43).
Its black bars were added by the test generator. The two supplied vocabularies
each contained 300 candidates, including the known answer. See
[all four outcomes](../OCR_RECOVERY.md#exploratory-known-answer-check).

The pixelation example uses the synthetic name “Harper Cedar,” 400 supplied
candidates, and a declared 5-pixel grid. It ranks the known fixture answer first,
while retaining 21 compatible candidates. It does not evaluate unknown-grid
detection, open-vocabulary search, or genuine redactions.

## Evidence files

- [`examples/evidence.json`](examples/evidence.json) records the displayed
  candidates and full-precision scores, source report hashes, input-image hashes,
  crop coordinates, and source paths.
- The three small PNGs in `examples/` contain the actual input crops. The source
  pixels were extracted without retouching. Figures scale them for display;
  pixelation uses nearest-neighbor enlargement.
- [`examples/viewer.json`](examples/viewer.json) records the captured project's
  hash, binary hash, context preview, and viewer settings.

The score data are **documentation excerpts**, not complete ranking reports or
valid inputs to `assess`. Original `runs/` paths are historical local provenance;
the full scan experiments are not included in a fresh checkout.

## Reproduce the synthetic pixelation example

From the repository root, after building the CLI, use a new output directory:

```sh
target/release/unredact benchmark --output runs/readme-demo
target/release/unredact solve --project runs/readme-demo --group mosaic \
  --candidates runs/readme-demo/candidates.json --limit 5 \
  --output runs/readme-mosaic.json
```

The published run used `runs/readme-examples-001/demo` and
`runs/readme-examples-001/mosaic-result.json`. Installed fonts and rendering
libraries can affect exact scores; the displayed binary and input hashes are
recorded in the evidence file.

## Render the documentation figures

The renderer uses the included crops and score excerpts, so it does not need the
original experiment directories. With Python Playwright and Chromium or Chrome:

```sh
python3 scripts/render_readme_examples.py
```

This regenerates `candidate-ranking.png` and `pixelation.png`. It does not rerun
the solver or replace the saved data. To capture the actual viewer as well, pass
`--review-project PATH` for a calibrated project with a registered target, and
optionally `--binary PATH`. The published viewer screenshot uses the archived
“and” case from `runs/ocr-recovery-review-001/ranking-four`.

The renderer opens only local files and, when capturing the viewer, its loopback
server. It closes the browser and server after capture and checks that the
project JSON is unchanged.
