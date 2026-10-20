# Visible-text neural typography prototype

This small CPU neural network ranks candidate font files from **visible text**.
It produces 64-dimensional font embeddings and compares them with a gallery
rendered from the supplied font catalog. It does not propose hidden identities.
Use suggestions to prioritize physical renderer calibration; do not use them as
a hard font filter until real-document recall is established.

The four convolutional stages use global mean and standard-deviation pooling.
Training minimizes normalized font classification loss, while inference uses
embedding retrieval. This allows a catalog to include font families absent from
training. It does not establish generalization to arbitrary fonts.

## Reproduce

An isolated CPU environment and the first trained model are in
`runs/neural-venv` and `runs/neural-001`. The font catalog contains exact file paths
and SHA-256 hashes. The initial run uses PyTorch 2.14.1+cpu, NumPy 2.5.2, and Pillow
12.3.0 on Python 3.14.8. Rendering provenance and text vocabularies are saved in
`manifest.json`. Installed package versions should be preserved when comparing
runs; Pillow rendering and platform libraries can affect pixels.

```sh
runs/neural-venv/bin/python scripts/neural/train.py \
  --output runs/neural-NEW --epochs 24 --samples 160 --threads 8
runs/neural-venv/bin/python -m unittest discover -s scripts/neural -p 'test_*.py' -v
```

Training uses 3,840 synthetic crops from 24 font faces. The checkpoint is selected
using 576 validation crops. Training, validation, and evaluation have disjoint
word vocabularies. Evaluation uses a 36-face candidate catalog, including 12 faces
from three families excluded from training, and 864 crops per degradation condition.
The same exact font binaries and Pillow renderer generate gallery and query data.
This evaluates synthetic font retrieval, not recovery from real redactions.

The standard simulation includes size changes, horizontal stretching, slight
rotation, blur, noise, thresholding, and JPEG compression. A separate shifted
condition increases stretch, skew, blur, noise, and compression. It is still
synthetic; scanned and photocopied originals can differ substantially.

Saved artifacts:

- `model.pt`: trained checkpoint, selected by validation only.
- `manifest.json`, `catalog.json`: configuration, splits, font hashes, environment.
- `training-data.npz`: exact generated training and validation arrays and labels.
- `history.json`: every epoch's loss and validation retrieval scores.
- `evaluation.json`: held-out results against a fixed, non-neural stroke/density
  descriptor; this is a simple comparator, not an established state of the art.
- `evaluation-*.npz`: exact evaluation pixels, labels, texts, and all candidate scores.
- `gallery.npz`: gallery embeddings and baseline features.

## JSON inference transport

```json
{
  "schema_version": 1,
  "model": "runs/neural-001/model.pt",
  "catalog": "runs/neural-001/catalog.json",
  "images": [
    {"path": "visible-line.png"},
    {"path": "page.png", "box": [120, 300, 900, 350]}
  ],
  "top_k": 10
}
```

`images` accepts paths or objects. Boxes are `[left, top, right, bottom]` in original
image pixels. Supply short visible lines or several words from consistent typography.
Long lines are center-cropped after normalizing text height. Redaction bars, rules,
logos, and mixed styles can corrupt normalization. No OCR is required.

`catalog` can also be an inline array of objects with `id`, `path`, and `sha256`.
Exact hashes are verified before inference. Files and JSON are local trusted inputs.

```sh
runs/neural-venv/bin/python scripts/neural/infer.py --input request.json
```

The command emits one JSON response, with `status`, `suggestions`, model hash,
input-image hashes, font hashes, and per-crop scores. Failed validation emits a
JSON error and nonzero exit status. Suggestions are sorted by **mean cosine
similarity**, not a calibrated probability. The model always ranks available
fonts, even when the correct font is absent; it cannot establish that the catalog
contains a match. Averaging related crops improves stability but must not be
interpreted as independent statistical evidence.

Rust calibration now invokes this JSON process with `--neural-config`, for example:

```sh
target/release/unredact calibrate --project runs/my-project --page PAGE_ID \
  --id body --size-group size-0 --neural-config runs/neural-001/calibration.json
```

The config contains paths relative to its own directory:

```json
{
  "python": "../neural-venv/bin/python",
  "script": "../../scripts/neural/infer.py",
  "model": "model.pt",
  "catalog": "catalog.json",
  "top_k": 8,
  "timeout_seconds": 120
}
```

Rust builds up to 24 montages containing only two or three distinct training
words per line, excluding redactions, held-out word identities, and overlapping
OCR boxes. Gaps are fresh white pixels. It saves these images and source boxes,
checks model/font/evidence hashes, and adds suggestions to the existing font
set. Physical width validation selects fits; audit text remains withheld. Model
scores never change redaction-candidate ranking. Profile provenance is saved in
`neural_search`. A later target intersecting these selection pixels is rejected.

This preserves alternative fonts but does not automatically split mixed-font
regions. A suitable OCR block or size group still matters. The Python environment
and model remain optional; the Rust solver can run without them. Real Senate
scan audits so far show no improvement over the default font set; details and
the full-catalog control are in the root README.

## Limits and next steps

Real scan accuracy, unfamiliar renderer behavior, historical font versions,
mixed-font regions, handwritten text, and unknown scripts are not established.
Input normalization removes absolute scale; this network identifies appearance,
not point size or kerning parameters. Those remain physical calibration tasks.
The next useful experiment is real scanned-text retrieval with known typography,
followed by an end-to-end calibration/ranking ablation.

## First trained model

The frozen `runs/neural-001/model.pt` has 93,016 parameters (374 KB). Validation
selected epoch 23 of 24; final test pixels did not influence checkpoint choice.
All rows rank the same 36 exact font faces and use held-out text.

| Synthetic condition | Crops | Neural top-1 | Neural top-5 | Simple descriptor top-1 |
|---|---:|---:|---:|---:|
| Standard, trained families | 576 | 96.7% | 100% | 24.3% |
| Standard, unseen families | 288 | 87.2% | 100% | 28.8% |
| Stronger degradation, trained families | 576 | 60.8% | 94.8% | 8.7% |
| Stronger degradation, unseen families | 288 | 54.5% | 95.5% | 11.8% |

The substantial degradation penalty argues for retaining alternative fonts and
measuring real-scan recall before using this model to exclude any candidates.
These results establish a learned font-retrieval prototype, not superiority over
the existing complete unredaction tools.

## Additional controls

After the training run finishes, freeze the selected weights and run:

```sh
runs/neural-venv/bin/python scripts/neural/evaluate_extra.py --run runs/neural-001
```

This evaluates a randomly initialized encoder, groups of six same-font visible
crops, and an independent Cairo renderer using each exact font file. Cairo uses
its text API and FreeType directly, with hinted and unhinted modes; Pillow training
uses its bundled FreeType/RAQM stack. Library versions and renderer source/binary
hashes are recorded. The Cairo images are clean raster samples, not physical scans.
No weights or ranking configuration are changed using these results.

`evaluation-extra.json` records the additional metrics. The pooled measurements
describe six correlated crops from the same font, not six independent font cases.
The random encoder control helps distinguish learned typography from useful
features supplied by architecture alone. These controls still cover only Latin
text and the chosen catalog; they do not establish open-world font identification.

Frozen-model Cairo results, using held-out text and the same 36-face gallery:

| Cairo query rendering | Crops | Neural top-1 | Neural top-5 | Random encoder top-1 |
|---|---:|---:|---:|---:|
| Unhinted, trained families | 288 | 97.9% | 100% | 37.8% |
| Unhinted, unseen families | 144 | 81.9% | 100% | 47.9% |
| Hinted, trained families | 288 | 96.5% | 100% | 38.9% |
| Hinted, unseen families | 144 | 82.6% | 100% | 42.4% |

Pooling six synthetic same-font crops reaches 100% top-1 in the standard condition
(96 trained-family groups and 48 unseen-family groups). Under stronger degradation,
pooled top-1 is 80.2% and 72.9%, respectively, with 100% top-5. These groups reuse
the individual-crop test set; they are an aggregation diagnostic, not another
independent test set. No tuning used the extra evaluation results.
