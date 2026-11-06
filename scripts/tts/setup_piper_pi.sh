#!/usr/bin/env bash
# Provision the Piper venv and the pinned voice. Safe to re-run: a voice file
# whose md5 already matches is kept, and pip skips satisfied requirements.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

RUNTIME_ROOT="${JARVIS_PIPER_RUNTIME_ROOT:-$HOME/.local/share/jarvis/piper}"
VENV_DIR="${JARVIS_PIPER_VENV_DIR:-$RUNTIME_ROOT/venv}"
VOICE_DIR="${JARVIS_PIPER_VOICE_DIR:-$RUNTIME_ROOT/voices}"

VOICE_NAME="en_GB-northern_english_male-medium"
VOICE_PATH="en/en_GB/northern_english_male/medium"
VOICES_BASE_URL="https://huggingface.co/rhasspy/piper-voices/resolve/main"
VOICES_JSON_URL="$VOICES_BASE_URL/voices.json"

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "error: setup_piper_pi.sh must run on Linux" >&2
    exit 1
fi

if [[ "$(uname -m)" != "aarch64" ]]; then
    echo "error: expected aarch64, got $(uname -m)" >&2
    exit 1
fi

mkdir -p "$RUNTIME_ROOT" "$VOICE_DIR"

if [[ ! -x "$VENV_DIR/bin/python" ]]; then
    echo "Creating Piper virtual environment..."
    python3 -m venv "$VENV_DIR"
fi

PYTHON="$VENV_DIR/bin/python"

echo "Installing locked Piper dependencies..."

"$PYTHON" -m pip install \
    --require-hashes \
    -r "$SCRIPT_DIR/piper_requirements.lock"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "Fetching voice checksums..."
curl --fail --silent --show-error --location \
    --output "$WORK_DIR/voices.json" "$VOICES_JSON_URL"

# Print "<md5> <file name>" for each file of the voice, read from voices.json.
expected_md5s() {
    "$PYTHON" -I - "$WORK_DIR/voices.json" "$VOICE_NAME" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    voice = json.load(source)[sys.argv[2]]

for path, meta in voice["files"].items():
    name = path.rsplit("/", 1)[-1]
    if name.endswith((".onnx", ".onnx.json")):
        print(meta["md5_digest"], name)
PY
}

file_md5() {
    "$PYTHON" -I - "$1" <<'PY'
import hashlib
import sys

digest = hashlib.md5()
with open(sys.argv[1], "rb") as source:
    for block in iter(lambda: source.read(1024 * 1024), b""):
        digest.update(block)
print(digest.hexdigest())
PY
}

EXPECTED="$(expected_md5s)"

if [[ "$(wc -l <<<"$EXPECTED")" -ne 2 ]]; then
    echo "error: voices.json does not list the expected files for $VOICE_NAME" >&2
    exit 1
fi

while read -r expected_md5 name; do
    target="$VOICE_DIR/$name"

    if [[ -f "$target" && "$(file_md5 "$target")" == "$expected_md5" ]]; then
        echo "Voice file already verified: $name"
        continue
    fi

    echo "Downloading $name..."
    curl --fail --silent --show-error --location \
        --output "$WORK_DIR/$name" "$VOICES_BASE_URL/$VOICE_PATH/$name"

    if [[ "$(file_md5 "$WORK_DIR/$name")" != "$expected_md5" ]]; then
        echo "error: md5 mismatch for $name" >&2
        exit 1
    fi

    mv "$WORK_DIR/$name" "$target"
done <<<"$EXPECTED"

echo
echo "Jarvis Piper setup complete."
echo "Python: $PYTHON"
echo "Model:  $VOICE_DIR/$VOICE_NAME.onnx"
