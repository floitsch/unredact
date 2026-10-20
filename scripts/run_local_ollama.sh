#!/bin/sh
# Copyright (C) 2026 Toit contributors.
# Isolated workspace storage and loopback binding; no account or cloud inference.
set -eu
unredact_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mkdir -p "$unredact_root/runs/ollama-local/models"
export OLLAMA_HOST=127.0.0.1:11435
export OLLAMA_MODELS="$unredact_root/runs/ollama-local/models"
export OLLAMA_NO_CLOUD=1
export OLLAMA_NUM_PARALLEL=1
export OLLAMA_CONTEXT_LENGTH=8192
exec ollama serve
