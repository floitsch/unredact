# Set up a fresh checkout

The repository contains application source, benchmark preparation/evaluation
scripts, the trained small font network, and compact final reports in
[`artifacts/`](../artifacts/README.md). The original prepared `runs/` directory is
not included. Historical documents retain its paths as experiment provenance.

## Rust application

Install the native dependencies listed in [Build](../README.md#build), then run:

```sh
cargo build --locked --release
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
target/release/unredact benchmark --output runs/demo
```

The core application does not require Python, a model service or API credentials.
Import requires Tesseract and the native PDF/font libraries described in Build.

## Optional local language models

Create a virtual environment at `runs/neural-venv` with CPU PyTorch, Transformers,
NumPy, Pillow and safetensors. The environment used for the published model is
recorded in `artifacts/neural-font-v1/requirements.lock`; its package versions and
Python version are historical provenance rather than a portable lock for every OS.
Use Transformers 4.57.6 for the included T5 providers. The working environment used
PyTorch 2.14.1+cpu, NumPy 2.5.2, Pillow 12.3.0 and safetensors 0.6.2 on Python 3.14.8.

Download the pinned T5 checkpoint once with the included standard-library script:

```sh
python3 scripts/fetch_span_model.py --output runs/span-model-001/t5-base
```

This public model requires no API token. Its approximately 892 MB weights are not
stored in Git. Then follow the [analysis quickstart](QUICKSTART.md). Local Ollama
providers for Llama/Gemma are described in the main README.

## Optional trained font network

The checkpoint is already included at `artifacts/neural-font-v1/model.pt`.
Generate a catalog of locally installed fonts with Fontconfig, using Python in
the repository root:

```python
import hashlib, json, subprocess
from pathlib import Path

paths = sorted(set(subprocess.check_output(
    ["fc-list", "--format", "%{file}\n"], text=True).splitlines()))
fonts, seen = [], set()
for path in paths:
    sha = hashlib.sha256(Path(path).read_bytes()).hexdigest()
    if sha in seen:
        continue
    seen.add(sha)
    fonts.append({"id": sha, "path": str(Path(path).resolve()), "sha256": sha})
if not 1 <= len(fonts) <= 1000:
    raise ValueError("Choose a catalog containing 1–1000 distinct font files")
Path("runs").mkdir(exist_ok=True)
Path("runs/font-catalog.json").write_text(json.dumps(fonts, indent=2) + "\n")
```

Save this configuration as `runs/neural-config.json`:

```json
{
  "python": "neural-venv/bin/python",
  "script": "../scripts/neural/infer.py",
  "model": "../artifacts/neural-font-v1/model.pt",
  "catalog": "font-catalog.json",
  "top_k": 8,
  "timeout_seconds": 120
}
```

Pass it to `calibrate --neural-config runs/neural-config.json`. The font network
uses visible words only; its suggestions supplement physical calibration. It is
optional and has not demonstrated a real-scan advantage over ordinary font search.

## External comparisons and larger fixtures

Baseline checkouts are excluded rather than vendored. The reviewed versions are:

- `Alex-Gilbert/unredact`: `196cc322d980666782e23a6e26aba9712ca4c6d1`.
- `mortenfj/unredactron`: `d00be632aa463355ec72158244e340310eba7253`.

Clone them into `baseline-alex` and `baseline-morten`, and check out those commits
before using the adapters. The [comparison instructions](COMPARISON.md) describe
the additional browser/WASM/Python dependencies and recorded adaptations.
The [benchmark protocol](BENCHMARK_EXPANSION.md) describes source downloads,
fixture generation, label review, document splits and evaluation. The ordinary
Rust tests do not require these external checkouts.
