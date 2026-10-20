# Local target review

Run the local viewer against an imported project:

```sh
cargo run --release -- review --project work/project --page PAGE_ID
```

Open the printed `http://127.0.0.1:PORT/SESSION/` URL in a browser. Omit
`--page` to use the first imported page. `--port 8080` chooses a fixed loopback
port; the default chooses an available port. Stop the server with Ctrl-C.
The viewer and all page data stay local. It uses no web services or browser
dependencies. The printed URL includes a random session key.

The viewer verifies the imported page's dimensions and gray-pixel SHA-256 before
displaying a lossless image. Overlays and zoom only affect the canvas display;
they do not alter evidence pixels, OCR, calibration, or project files.

1. Choose a registered target, proposed mask, or **Draw new rectangle**. The pink
   rectangle is an editable draft. Drag it to move it, drag its corner handles
   to resize it, or enter coordinates in original page pixels.
2. Select a profile calibrated for this page. Colored overlays mark that
   profile's training, validation, audit, spacing, and neural font-selection
   exclusions. Orange dashed rectangles are proposed masks; red dotted
   rectangles are registered targets. Layer checkboxes control visibility.
3. Check the ID, group hypothesis, surface kind, dependence key, and evidence
   JSON. New drafts default to `opaque` evidence with zero padding and a
   one-pixel measurement sigma. These are explicit assumptions to review, not
   estimated padding or uncertainty. Existing targets retain their evidence.
4. Inspect the context preview and validation errors. The server calls the same
   Rust same-row or paragraph context extractor used by `context-rank`, with
   the draft replacing an existing target of the same ID. Cyan outlines show
   context OCR words. Supplied context overrides are labeled in the preview.
   Context extraction alone does not establish that a target is valid.
5. **Download validated target JSON** exports one target specification. The
   server uses the same registration checks as `target`, including calibration
   exclusions and scan-anchor pixel checks, and rereads the project and verifies
   the page image again before export. Downloading does not save the project.

Register a downloaded target explicitly:

```sh
target/release/unredact target --project work/project --spec target-1.json
```

Registering an existing target ID replaces that target. The registration command
validates it again. Restart review after modifying the project from another
process so its visible overlays and preview match the new project state.

Pages without calibrated profiles can be inspected and drafted, but cannot
export a valid target until a profile is calibrated. Use `inspect` to list page
IDs and consult [QUICKSTART.md](QUICKSTART.md) for import and calibration.

## Explicit ranking preset

The benchmark-selected paragraph-context, whole-sequence-score, physical-weight
1 configuration is available under a named, versioned preset:

```sh
target/release/unredact context-rank \
  --project work/project --group GROUP_ID --candidates candidates.txt \
  --ranking-preset paragraph-sequence-physical-v1 \
  --provider runs/neural-venv/bin/python \
  --provider-arg scripts/span_context_provider.py \
  --provider-arg=--model-dir=runs/span-model-001/t5-base \
  --output ranked.json
```

The preset is equivalent to `--paragraph-context --sequence-score
--physical-weight 1`. It is opt-in; existing defaults remain available. The CLI
rejects combining it with `--paragraph-context`, `--sequence-score`,
`--word-normalization`, or `--physical-weight`, even when the explicit value
matches. Provider, model, batching, and timeout options stay independent.

The preset names scoring settings; it does not guarantee an answer or calibrate
model scores as probabilities. See [RESULTS.md](RESULTS.md) for the evaluation's
scope and limitations.

## Verify the viewer

`cargo test review` covers pixel fidelity, fingerprint checks, registration
validation, request bounds, and local request isolation. `cargo test --test
review_cli` checks the preset conflicts and CLI help. With Python Playwright and
Chromium or Chrome installed, run the browser interaction test:

```sh
cargo build
python3 scripts/test_local_review.py --screenshot /tmp/unredact-review.png
```

The smoke test draws, moves, and resizes a draft; checks context and calibration
errors; downloads and registers a target; and verifies that imported HTML syntax
is displayed as text. It creates a temporary project and binds loopback only.
