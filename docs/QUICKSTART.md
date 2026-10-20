# Run a local analysis

Run these commands from the repository root after installing the native
dependencies listed in [Build](../README.md#build). The prepared workspace already
contains the Python environment and pinned T5 weights used below. In a fresh
workspace, follow [local model setup](../README.md#offline-variable-length-span-completion)
first. No model API token is required.

## Import and locate the masks

```sh
cargo build --release
target/release/unredact import --project runs/my-document --dpi 240 document.pdf
target/release/unredact inspect --project runs/my-document
```

The inspection report contains page IDs, OCR blocks and proposed masks. View the
page image named in `runs/my-document/project.json` and record mask rectangles in
that image's pixel coordinates. Multiple PDFs can be supplied to `import`.

Calibrate an initial body-text profile, replacing `PAGE_ID` with an actual ID:

```sh
target/release/unredact calibrate --project runs/my-document \
  --page PAGE_ID --id body
```

For mixed typography, use `--block BLOCK_ID` or the size-group workflow in the
[full instructions](../README.md#analyze-documents).

## Record the target and candidate text

Save `target.json`, replacing the page ID and rectangle with actual measurements:

```json
{
  "id": "occurrence-1",
  "page": "PAGE_ID",
  "bounds": {"x": 100, "y": 250, "w": 160, "h": 30},
  "profile": "body",
  "group": "hypothesis-1",
  "surface_kind": "canonical",
  "evidence": {
    "kind": "opaque",
    "padding_min": 0,
    "padding_max": 12,
    "measurement_sigma": 1.5
  },
  "left_context": "",
  "right_context": "",
  "dependence_key": null
}
```

Padding is the assumed **total left-plus-right padding**, in pixels. Choose an
interval appropriate to the mask; the example values are assumptions. Empty
context fields let the app select visible OCR context.

```sh
target/release/unredact target --project runs/my-document --spec target.json
target/release/unredact calibrate --project runs/my-document \
  --page PAGE_ID --id body
```

Record every known mask on the page, then repeat calibration so words inside
those targets are excluded. Reuse the same block/size-group options when
recalibrating. Give occurrences the same group only when testing an explicit
shared-identity hypothesis. Use a shared `dependence_key` for copied observations.

Create `candidates.txt` with one complete candidate per line. Alternatively, use
JSON with identities and aliases; [names and AI hooks](../README.md#names-and-ai-hooks)
describes the schema and visible-corpus harvesting. The public-role starter list
is `data/public-name-seeds.json`; its entries carry source attribution rather than
inferred redaction identities.

## Rank with local T5 and physical evidence

```sh
target/release/unredact context-rank \
  --project runs/my-document --group hypothesis-1 \
  --candidates candidates.txt --paragraph-context --sequence-score \
  --physical-weight 1 \
  --provider runs/neural-venv/bin/python \
  --provider-arg scripts/span_context_provider.py \
  --provider-arg=--model-dir=runs/span-model-001/t5-base \
  --output runs/my-ranking.json
```

This is the selected configuration in the multidocument language comparison.
The default CLI physical weight remains zero; the explicit flag matters.
The report retains physical costs, language scores, context provenance and ties.
If no visible context is available, supply accurate visible context in the target
specification or use `solve` for a geometry-only analysis.

To propose additional candidates from context, run:

```sh
target/release/unredact expand \
  --project runs/my-document --group hypothesis-1 \
  --candidates candidates.txt --paragraph-context \
  --provider runs/neural-venv/bin/python \
  --provider-arg scripts/span_proposal_provider.py \
  --provider-arg=--model-dir=runs/span-model-001/t5-base \
  --output runs/expanded-candidates.json
```

Pass `runs/expanded-candidates.json` to the ranking command to rescore the expanded
vocabulary. Proposal expansion can improve coverage or introduce stronger wrong
alternatives; inspect its results separately from ranking. Local Llama/Gemma
proposal adapters are also described in the full instructions.

## Pixelated regions

For retained mosaic information, import the pixelated document and calibrate its
visible text outside all supplied regions. Use `rank-pixelation` with that project.
Save all masks on the page in `regions.json` as an array of rectangles:

```json
[{"x": 100, "y": 250, "w": 160, "h": 30}]
```

```sh
target/release/unredact rank-pixelation \
  --project runs/my-document --profile body --regions regions.json \
  --target-index 0 --candidates candidates.txt --learn-vertical \
  --output runs/my-pixelation-ranking.json
```

Use a profile calibrated for the same page, without `--learn-spaces` or hinted
metrics. This command searches shared page grids and includes partial cells.
[Pixelation instructions](../README.md#pixelated-text) cover
diagnostics and combining explicitly linked reports across PDFs.

All results rank hypotheses within the supplied vocabulary. A first-place result
does not verify the original text, and the correct answer may be absent. Consult
the [benchmark results](BENCHMARK_EXPANSION.md) for measured performance and scope.
