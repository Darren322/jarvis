#!/usr/bin/env python3

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
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


def write_manifest(
    path: Path,
    lock: dict,
) -> None:
    manifest = {
        "schema_version": 1,
        "engine": lock["supertonic"]["model"],
        "precision": lock["supertonic"]["precision"],
        "package_id": lock["supertonic"]["package_id"],
        "voice": lock["voice"]["name"],
        "sid": lock["voice"]["sid"],
        "files": {},
    }

    for filename, metadata in lock["supertonic"]["files"].items():
        manifest["files"][filename] = {
            "sha256": metadata["sha256"],
        }

    manifest["files"]["voice.bin"] = {
        "sha256": lock["voice"]["generated"]["sha256"],
    }

    with path.open("w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")


def verify_staged_bundle(
    staging_dir: Path,
    lock: dict,
) -> None:
    for filename, metadata in lock["supertonic"]["files"].items():
        verify_file(
            staging_dir / filename,
            metadata["sha256"],
        )

    verify_file(
        staging_dir / "voice.bin",
        lock["voice"]["generated"]["sha256"],
    )


def main() -> int:
    args = parse_args()

    with LOCK_PATH.open("r", encoding="utf-8") as f:
        lock = json.load(f)

    # Verify the existing research/source assets before staging anything.
    for filename, metadata in lock["supertonic"]["files"].items():
        verify_file(
            args.model_dir / filename,
            metadata["sha256"],
        )

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

    output_parent = args.output_dir.parent
    output_parent.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(
        prefix=".jarvis-tts-stage-",
        dir=output_parent,
    ) as temp:
        staging_dir = Path(temp) / "bundle"
        staging_dir.mkdir()

        # Copy only the locked runtime model files.
        for filename in lock["supertonic"]["files"]:
            shutil.copy2(
                args.model_dir / filename,
                staging_dir / filename,
            )

        converter_path = Path(temp) / "generate_voices_bin.py"

        download_file(
            converter_url,
            converter_path,
        )

        verify_file(
            converter_path,
            converter["sha256"],
        )

        generate_voice_bin(
            args.m5_json,
            converter_path,
            staging_dir / "voice.bin",
        )

        # Nothing is promoted until the complete staged bundle verifies.
        verify_staged_bundle(
            staging_dir,
            lock,
        )

        write_manifest(
            staging_dir / "manifest.json",
            lock,
        )

        # Replace a previously prepared runtime bundle only after staging
        # has completed successfully.
        if args.output_dir.exists():
            if args.output_dir.is_dir():
                shutil.rmtree(args.output_dir)
            else:
                args.output_dir.unlink()

        staging_dir.rename(args.output_dir)

    print("Verified Supertonic INT8 source assets.")
    print("Verified official M5.json.")
    print("Verified sherpa-onnx voice converter.")
    print("Verified complete staged runtime bundle.")
    print(f"Prepared runtime bundle: {args.output_dir}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
