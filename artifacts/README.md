# Published artifacts

This directory contains the trained font checkpoint and compact final result
reports. `manifest.json` records their original workspace paths and SHA-256 hashes.
The copies are byte-identical to the originals.

- `neural-font-v1/model.pt`: the trained 93,016-parameter font network.
- `neural-font-v1/`: training configuration, history, evaluation and environment
  metadata. The recorded font catalog describes the training machine's fonts.
- `evaluation/language.json`: complete held-out language results and CLI audit.
- `evaluation/native-confidence.json` and `native-paired.json`: all fixed native
  confidence controls and their paired comparisons.
- `evaluation/families.json`: registered publisher-family sensitivity.
- `evaluation/geometry.json` and `pixelation.json`: complete result summaries.
- `evaluation/*audit.json`, `completion.json`, `requirements.json`: completion
  evidence and links to original inputs.

The embedded manifests retain paths from the original experiment workspace.
Most files referenced under `runs/` are not distributed here. These selected
reports preserve the recorded outcomes; they do not replace the complete frozen
inputs needed to rerun every hash verifier. Large datasets, source PDFs, Python
environments, third-party model downloads, build products and baseline checkouts
are excluded from Git. Preparation and evaluation source is included in `scripts/`.

See [fresh-checkout setup](../docs/CHECKOUT_SETUP.md) before following instructions
that refer to the original prepared workspace.
