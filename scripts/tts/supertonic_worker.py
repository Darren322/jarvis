#!/usr/bin/env python3

import argparse
import contextlib
import ctypes
import hashlib
import json
import os
import sys
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path

PROTOCOL_VERSION = 1
MAX_TEXT_BYTES = 4096
EXPECTED_SAMPLE_RATE = 44100
MAX_AUDIO_SAMPLES = 5_292_000
MAX_WAV_BYTES = 16 * 1024 * 1024
MAX_REQUEST_BYTES = 32 * 1024
MAX_RESPONSE_BYTES = 4 * 1024
EXPECTED_PACKAGES = {
    "sherpa-onnx": "1.13.8",
    "sherpa-onnx-core": "1.13.8",
}
EXPECTED_MODEL_FILES = {
    "text_encoder.int8.onnx",
    "duration_predictor.int8.onnx",
    "vector_estimator.int8.onnx",
    "vocoder.int8.onnx",
    "tts.json",
    "unicode_indexer.bin",
}
MAX_THREADS = 4


class WorkerFrameError(Exception):
    def __init__(self, category: str):
        self.category = category
        super().__init__(category)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument("--voice-style", required=True, type=Path)
    parser.add_argument("--threads", type=int, default=2)
    args = parser.parse_args()

    if args.threads < 1 or args.threads > MAX_THREADS:
        parser.error(f"--threads must be between 1 and {MAX_THREADS}")

    return args


@contextlib.contextmanager
def redirect_process_stdout_to_stderr():
    """Keep Python and native-library chatter off the protocol pipe."""
    sys.stdout.flush()
    stdout_fd = sys.stdout.fileno()
    saved_stdout_fd = os.dup(stdout_fd)

    try:
        os.dup2(sys.stderr.fileno(), stdout_fd)
        yield
    finally:
        sys.stdout.flush()
        try:
            ctypes.CDLL(None).fflush(None)
        except (AttributeError, OSError):
            pass
        os.dup2(saved_stdout_fd, stdout_fd)
        os.close(saved_stdout_fd)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()

    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)

    return digest.hexdigest()


def read_json(path: Path) -> dict:
    with path.open("r", encoding="utf-8") as source:
        value = json.load(source)

    if not isinstance(value, dict):
        raise RuntimeError("invalid TTS asset metadata")

    return value


def verify_file(path: Path, expected_sha256: str) -> None:
    if not path.is_file() or sha256_file(path) != expected_sha256:
        raise RuntimeError("TTS asset verification failed")


def verify_bundle(args: argparse.Namespace) -> dict:
    script_dir = Path(__file__).resolve().parent
    lock = read_json(script_dir / "assets.lock.json")
    model = lock.get("supertonic")
    voice = lock.get("voice")

    if (
        lock.get("schema_version") != 1
        or not isinstance(model, dict)
        or not isinstance(voice, dict)
        or model.get("model") != "supertonic-3"
        or model.get("precision") != "int8"
        or not isinstance(model.get("package_id"), str)
        or not model.get("package_id")
        or voice.get("name") != "M5"
        or voice.get("sid") != 0
    ):
        raise RuntimeError("TTS asset lock does not match the selected configuration")

    model_files = model.get("files")
    generated_voice = voice.get("generated")

    if (
        not isinstance(model_files, dict)
        or set(model_files) != EXPECTED_MODEL_FILES
        or not isinstance(generated_voice, dict)
        or generated_voice.get("file") != "voice.bin"
    ):
        raise RuntimeError("TTS asset lock has an unexpected file set")

    expected_files = {
        filename: metadata.get("sha256")
        for filename, metadata in model_files.items()
        if isinstance(metadata, dict)
    }
    if len(expected_files) != len(model_files):
        raise RuntimeError("TTS asset lock is malformed")
    expected_files["voice.bin"] = generated_voice.get("sha256")

    if any(not isinstance(digest, str) or len(digest) != 64 for digest in expected_files.values()):
        raise RuntimeError("TTS asset lock is malformed")

    model_dir = args.model_dir.resolve(strict=True)
    manifest = read_json(model_dir / "manifest.json")
    manifest_files = manifest.get("files")

    if (
        manifest.get("schema_version") != 1
        or manifest.get("engine") != model["model"]
        or manifest.get("precision") != model["precision"]
        or manifest.get("package_id") != model.get("package_id")
        or manifest.get("voice") != voice["name"]
        or manifest.get("sid") != voice["sid"]
        or not isinstance(manifest_files, dict)
        or set(manifest_files) != set(expected_files)
    ):
        raise RuntimeError("prepared TTS manifest does not match the asset lock")

    for filename, expected_sha256 in expected_files.items():
        entry = manifest_files.get(filename)
        if not isinstance(entry, dict) or entry.get("sha256") != expected_sha256:
            raise RuntimeError("prepared TTS manifest does not match the asset lock")
        verify_file(model_dir / filename, expected_sha256)

    voice_path = args.voice_style.resolve(strict=True)
    if voice_path != (model_dir / "voice.bin").resolve(strict=True):
        raise RuntimeError("TTS worker must use the prepared M5 voice")

    return manifest


def load_runtime():
    for package, expected in EXPECTED_PACKAGES.items():
        try:
            installed = version(package)
        except PackageNotFoundError as error:
            raise RuntimeError("required TTS package is missing") from error

        if installed != expected:
            raise RuntimeError("installed TTS package version is unsupported")

    import numpy as np
    import sherpa_onnx
    import soundfile as sf

    return np, sherpa_onnx, sf


def load_tts(args: argparse.Namespace, sherpa_onnx):
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
    config.model.supertonic.voice_style = os.fspath(args.voice_style)

    config.model.num_threads = args.threads
    config.model.provider = "cpu"

    if not config.validate():
        raise RuntimeError("invalid Supertonic configuration")

    return sherpa_onnx.OfflineTts(config)


def emit_frame(frame: dict) -> None:
    encoded = json.dumps(frame, separators=(",", ":")).encode("utf-8")

    if len(encoded) + 1 > MAX_RESPONSE_BYTES:
        encoded = json.dumps(
            error_result(None, "response_too_large"),
            separators=(",", ":"),
        ).encode("utf-8")

    sys.stdout.buffer.write(encoded + b"\n")
    sys.stdout.buffer.flush()


def error_result(operation_id, error):
    return {
        "type": "error",
        "protocol": PROTOCOL_VERSION,
        "id": operation_id,
        "error": error,
    }


def read_request_line(reader):
    raw = reader.readline(MAX_REQUEST_BYTES + 1)

    if not raw:
        return None

    if len(raw) > MAX_REQUEST_BYTES:
        raise WorkerFrameError("request_too_large")

    if not raw.endswith(b"\n"):
        raise WorkerFrameError("invalid_request")

    try:
        return raw[:-1].decode("utf-8")
    except UnicodeDecodeError:
        raise WorkerFrameError("invalid_utf8") from None


def bounded_operation_id(request):
    operation_id = request.get("id")
    if isinstance(operation_id, str):
        try:
            if 0 < len(operation_id.encode("utf-8")) <= 128:
                return operation_id
        except UnicodeEncodeError:
            pass
    return None


def handle_request(tts, request: dict, np, sf, sherpa_onnx) -> dict:
    operation_id = bounded_operation_id(request)

    if request.get("protocol") != PROTOCOL_VERSION:
        return error_result(operation_id, "unsupported_protocol")

    if request.get("type") != "speak":
        return error_result(operation_id, "unsupported_request")

    if operation_id is None:
        return error_result(None, "invalid_id")

    text = request.get("text")

    if not isinstance(text, str) or not text.strip():
        return error_result(operation_id, "invalid_text")

    try:
        text_bytes = text.encode("utf-8")
    except UnicodeEncodeError:
        return error_result(operation_id, "invalid_text")

    if len(text_bytes) > MAX_TEXT_BYTES:
        return error_result(operation_id, "text_too_large")

    output_path = request.get("output_path")

    if (
        not isinstance(output_path, str)
        or not output_path
        or not os.path.isabs(output_path)
        or not output_path.endswith(".wav")
    ):
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
    manifest = verify_bundle(args)

    with redirect_process_stdout_to_stderr():
        np, sherpa_onnx, sf = load_runtime()
        tts = load_tts(args, sherpa_onnx)

    emit_frame(
        {
            "type": "ready",
            "protocol": PROTOCOL_VERSION,
            "pid": os.getpid(),
            "engine": manifest["engine"],
            "precision": manifest["precision"],
            "voice": manifest["voice"],
            "language": "en",
            "provider": "cpu",
            "threads": args.threads,
            "sample_rate": tts.sample_rate,
            "num_speakers": tts.num_speakers,
        }
    )

    reader = sys.stdin.buffer
    while True:
        try:
            line = read_request_line(reader)
        except WorkerFrameError as error:
            emit_frame(error_result(None, error.category))
            return 1

        if line is None:
            break

        if not line:
            continue

        operation_id = None
        try:
            request = json.loads(line)
            if not isinstance(request, dict):
                result = error_result(None, "invalid_request")
            else:
                operation_id = bounded_operation_id(request)
                with redirect_process_stdout_to_stderr():
                    result = handle_request(tts, request, np, sf, sherpa_onnx)
        except json.JSONDecodeError:
            result = error_result(None, "invalid_json")
        except Exception:
            result = error_result(operation_id, "worker_error")

        emit_frame(result)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
