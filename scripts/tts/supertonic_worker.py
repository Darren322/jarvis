#!/usr/bin/env python3

import argparse
import json
import os
import sys
import numpy as np
import soundfile as sf

import sherpa_onnx

PROTOCOL_VERSION = 1
MAX_TEXT_BYTES = 4096
EXPECTED_SAMPLE_RATE = 44100
MAX_AUDIO_SAMPLES = 5_292_000
MAX_WAV_BYTES = 16 * 1024 * 1024
MAX_REQUEST_BYTES = 32 * 1024
MAX_RESPONSE_BYTES = 4 * 1024


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--voice-style", required=True)
    parser.add_argument("--threads", type=int, default=2)
    return parser.parse_args()


def emit_frame(frame: dict) -> None:
    encoded = json.dumps(frame, separators=(",", ":"))

    if len(encoded.encode("utf-8")) > MAX_RESPONSE_BYTES:
        encoded = json.dumps(
            error_result(None, "response_too_large"),
            separators=(",", ":"),
        )

    print(encoded, flush=True)


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


def error_result(operation_id, error):
    return {
        "type": "error",
        "protocol": PROTOCOL_VERSION,
        "id": operation_id,
        "error": error,
    }


def handle_request(tts: sherpa_onnx.OfflineTts, request: dict) -> dict:
    operation_id = request.get("id")

    if request.get("protocol") != PROTOCOL_VERSION:
        return error_result(operation_id, "unsupported_protocol")

    if request.get("type") != "speak":
        return error_result(operation_id, "unsupported_request")

    if not isinstance(operation_id, str) or not operation_id:
        return error_result(None, "invalid_id")

    text = request.get("text")

    if not isinstance(text, str) or not text.strip():
        return error_result(operation_id, "invalid_text")

    if len(text.encode("utf-8")) > MAX_TEXT_BYTES:
        return error_result(operation_id, "text_too_large")

    output_path = request.get("output_path")

    if not isinstance(output_path, str) or not output_path:
        return error_result(operation_id, "invalid_output_path")

    if not os.path.isabs(output_path):
        return error_result(operation_id, "invalid_output_path")

    generation = sherpa_onnx.GenerationConfig()
    generation.sid = 0
    generation.speed = 1.0
    generation.num_steps = 8
    generation.extra = {"lang": "en"}

    audio = tts.generate(text, generation)

    samples = np.asarray(audio.samples, dtype=np.float32)
    sample_rate = int(audio.sample_rate)

    if samples.ndim != 1:
        return error_result(operation_id, "invalid_audio")

    if samples.size == 0 or samples.size > MAX_AUDIO_SAMPLES:
        return error_result(operation_id, "invalid_audio")

    if not np.all(np.isfinite(samples)):
        return error_result(operation_id, "invalid_audio")

    if sample_rate != EXPECTED_SAMPLE_RATE:
        return error_result(operation_id, "invalid_audio")

    sf.write(
        output_path,
        samples,
        sample_rate,
        subtype="PCM_16",
        format="WAV",
    )
    wav_size = os.path.getsize(output_path)

    if wav_size > MAX_WAV_BYTES:
        os.remove(output_path)
        return error_result(operation_id, "invalid_audio")

    return {
        "type": "completed",
        "protocol": PROTOCOL_VERSION,
        "id": operation_id,
        "sample_rate": sample_rate,
        "num_samples": int(samples.size),
        "wav_bytes": wav_size,
    }


def main() -> int:
    args = parse_args()

    tts = load_tts(args)

    ready = {
        "type": "ready",
        "protocol": PROTOCOL_VERSION,
        "pid": os.getpid(),
        "engine": "supertonic-3",
        "precision": "int8",
        "voice": "M5",
        "language": "en",
        "provider": "cpu",
        "threads": args.threads,
        "sample_rate": tts.sample_rate,
        "num_speakers": tts.num_speakers,
    }

    emit_frame(ready)

    for line in sys.stdin:
        if len(line.encode("utf-8")) > MAX_REQUEST_BYTES:
            result = error_result(None, "request_too_large")
            emit_frame(result)
            continue

        line = line.strip()

        if not line:
            continue

        try:
            request = json.loads(line)

            if not isinstance(request, dict):
                result = error_result(None, "invalid_request")
            else:
                result = handle_request(tts, request)

        except json.JSONDecodeError:
            result = error_result(None, "invalid_json")

        except Exception:
            result = error_result(None, "worker_error")

        emit_frame(result)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

