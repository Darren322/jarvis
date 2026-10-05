#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

VENV_DIR="${JARVIS_TTS_VENV_DIR:-$HOME/.local/share/jarvis/tts/venv}"
RUNTIME_ROOT="${JARVIS_TTS_RUNTIME_ROOT:-$HOME/.local/share/jarvis/tts}"
MODEL_DIR="${JARVIS_TTS_SOURCE_MODEL_DIR:-}"
M5_JSON="${JARVIS_TTS_SOURCE_M5_JSON:-}"

PREPARED_MODEL_DIR="$RUNTIME_ROOT/models/supertonic-3-int8-m5"

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "error: setup_pi.sh must run on Linux" >&2
    exit 1
fi

if [[ "$(uname -m)" != "aarch64" ]]; then
    echo "error: expected aarch64, got $(uname -m)" >&2
    exit 1
fi

if [[ -z "$MODEL_DIR" ]]; then
    echo "error: JARVIS_TTS_SOURCE_MODEL_DIR is required" >&2
    exit 1
fi

if [[ -z "$M5_JSON" ]]; then
    echo "error: JARVIS_TTS_SOURCE_M5_JSON is required" >&2
    exit 1
fi

if [[ ! -d "$MODEL_DIR" ]]; then
    echo "error: model directory does not exist: $MODEL_DIR" >&2
    exit 1
fi

if [[ ! -f "$M5_JSON" ]]; then
    echo "error: M5.json does not exist: $M5_JSON" >&2
    exit 1
fi

mkdir -p "$RUNTIME_ROOT"

if [[ ! -x "$VENV_DIR/bin/python" ]]; then
    echo "Creating TTS virtual environment..."
    python3 -m venv "$VENV_DIR"
fi

PYTHON="$VENV_DIR/bin/python"

echo "Installing locked TTS dependencies..."

"$PYTHON" -m pip install \
    --require-hashes \
    -r "$SCRIPT_DIR/requirements.lock"

echo "Preparing verified Supertonic runtime assets..."

"$PYTHON" "$SCRIPT_DIR/prepare_assets.py" \
    --model-dir "$MODEL_DIR" \
    --m5-json "$M5_JSON" \
    --output-dir "$PREPARED_MODEL_DIR"

echo
echo "Jarvis TTS setup complete."
echo "Python: $PYTHON"
echo "Model:  $PREPARED_MODEL_DIR"
