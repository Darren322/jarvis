#!/usr/bin/env python3

import argparse
import json
import os
import sys

import sherpa_onnx


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--voice-style", required=True)
    parser.add_argument("--threads", type=int, default=2)
    return parser.parse_args()


def load_tts(args: argparse.Namespace) -> sherpa_onnx.OfflineTts:
    model_dir = args.model_dir

    config = sherpa_onnx.OfflineTtsConfig()

    config.model.supertonic.text_encoder = os.path.join(
        model_dir, "text_encoder.int8.onnx"
    )
    config.model.supertonic.duration_predictor = os.path.join(
        model_dir, "duration_predictor.int8.onnx"
    )
    config.model.supertonic.vector_estimator = os.path.join(
        model_dir, "vector_estimator.int8.onnx"
    )
    config.model.supertonic.vocoder = os.path.join(model_dir, "vocoder.int8.onnx")
    config.model.supertonic.tts_json = os.path.join(model_dir, "tts.json")
    config.model.supertonic.unicode_indexer = os.path.join(
        model_dir, "unicode_indexer.bin"
    )
    config.model.supertonic.voice_style = args.voice_style

    config.model.num_threads = args.threads
    config.model.provider = "cpu"

    if not config.validate():
        raise RuntimeError("invalid Supertonic configuration")

    return sherpa_onnx.OfflineTts(config)


def main() -> int:
    args = parse_args()

    tts = load_tts(args)

    ready = {
        "type": "ready",
        "pid": os.getpid(),
        "sample_rate": tts.sample_rate,
        "num_speakers": tts.num_speakers,
    }

    print(json.dumps(ready), flush=True)

    # Step 2A only: retain the loaded model until stdin closes.
    for _ in sys.stdin:
        pass

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
