#!/usr/bin/env python3

import sys
import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile
import urllib.request
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
LOCK_PATH = SCRIPT_DIR / "assets.lock.json"

CONVERTER_URL_TEMPLATE = (
    "https://raw.githubusercontent.com/{repository}/{version}/{file}"
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument("--m5-json", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()

    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)

    return digest.hexdigest()


def verify_file(path: Path, expected_sha256: str) -> None:
    if not path.is_file():
        raise RuntimeError(f"missing required asset: {path}")

    actual = sha256_file(path)

    if actual != expected_sha256:
        raise RuntimeError(f"asset hash mismatch: {path.name}")


def download_file(url: str, destination: Path) -> None:
    with urllib.request.urlopen(url, timeout=30) as response:
        with destination.open("wb") as output:
            shutil.copyfileobj(response, output)


def generate_voice_bin(
    m5_json: Path,
    converter_path: Path,
    output_path: Path,
) -> None:
    with tempfile.TemporaryDirectory(prefix="jarvis-m5-") as temp:
        voice_dir = Path(temp) / "voice_styles"
        voice_dir.mkdir()

        shutil.copy2(m5_json, voice_dir / "M5.json")

        subprocess.run(
            [
                sys.executable,
                str(converter_path),
                str(voice_dir),
                str(output_path),
            ],
            check=True,
        )


def main() -> int:
    args = parse_args()

    with LOCK_PATH.open("r", encoding="utf-8") as f:
        lock = json.load(f)

    # Verify the existing INT8 model bundle.
    for filename, metadata in lock["supertonic"]["files"].items():
        verify_file(
            args.model_dir / filename,
            metadata["sha256"],
        )

    # Verify the official M5 source before using it.
    verify_file(
        args.m5_json,
        lock["voice"]["source"]["sha256"],
    )

    converter = lock["voice"]["converter"]

    converter_url = CONVERTER_URL_TEMPLATE.format(
        repository=converter["repository"],
        version=converter["version"],
        file=converter["file"],
    )

    args.output_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="jarvis-converter-") as temp:
        converter_path = Path(temp) / "generate_voices_bin.py"

        download_file(converter_url, converter_path)

        verify_file(
            converter_path,
            converter["sha256"],
        )

        staged_voice = Path(temp) / "voice.bin"

        generate_voice_bin(
            args.m5_json,
            converter_path,
            staged_voice,
        )

        verify_file(
            staged_voice,
            lock["voice"]["generated"]["sha256"],
        )

        final_voice = args.output_dir / "voice.bin"
        shutil.copy2(staged_voice, final_voice)

    print("Verified Supertonic INT8 assets.")
    print("Verified official M5.json.")
    print("Verified sherpa-onnx voice converter.")
    print(f"Prepared M5-only voice.bin: {final_voice}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
