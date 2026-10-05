#!/usr/bin/env python3

import argparse
import hashlib
import json
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
LOCK_PATH = SCRIPT_DIR / "assets.lock.json"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument("--m5-json", required=True, type=Path)
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


def main() -> int:
    args = parse_args()

    with LOCK_PATH.open("r", encoding="utf-8") as f:
        lock = json.load(f)

    for filename, metadata in lock["supertonic"]["files"].items():
        verify_file(
            args.model_dir / filename,
            metadata["sha256"],
        )

    verify_file(
        args.m5_json,
        lock["voice"]["source"]["sha256"],
    )

    print("Verified Supertonic INT8 assets and official M5.json.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
