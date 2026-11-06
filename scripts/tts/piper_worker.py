#!/usr/bin/env python3
"""Piper speech worker speaking the same JSON-lines protocol as supertonic_worker.py.

The framing, limits and error codes are mirrored from that file on purpose: the
two workers share a wire format, not code. Piper writes at the voice's native
sample rate (22,050 Hz for the medium voices). Nothing is resampled here; the
rate is declared in the ready frame and in every completed response.
"""

import argparse
import contextlib
import ctypes
import json
import os
import sys
import wave
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path

PROTOCOL_VERSION = 1
MAX_TEXT_BYTES = 4096
MAX_AUDIO_SECONDS = 120
MAX_WAV_BYTES = 16 * 1024 * 1024
MAX_REQUEST_BYTES = 32 * 1024
MAX_RESPONSE_BYTES = 4 * 1024
MAX_OPERATION_ID_BYTES = 128
EXPECTED_PACKAGES = {
    "piper-tts": "1.8.0",
    "onnxruntime": "1.31.1",
}
DEFAULT_THREADS = 4
MAX_THREADS = 4
EXPECTED_SAMPLE_WIDTH_BYTES = 2
EXPECTED_CHANNELS = 1
ENGINE_NAME = "piper"
PRECISION_NAME = "fp32"


class WorkerFrameError(Exception):
    def __init__(self, category: str):
        self.category = category
        super().__init__(category)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--threads", type=int, default=DEFAULT_THREADS)
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


def load_runtime():
    for package, expected in EXPECTED_PACKAGES.items():
        try:
            installed = version(package)
        except PackageNotFoundError as error:
            raise RuntimeError("required TTS package is missing") from error

        if installed != expected:
            raise RuntimeError("installed TTS package version is unsupported")

    import onnxruntime
    from piper import PiperVoice
    from piper.config import PiperConfig
    from piper.voice import ESPEAK_DATA_DIR

    return onnxruntime, PiperVoice, PiperConfig, ESPEAK_DATA_DIR


def load_voice(args: argparse.Namespace, runtime):
    onnxruntime, PiperVoice, PiperConfig, espeak_data_dir = runtime

    model_path = args.model.resolve(strict=True)
    config_path = model_path.with_name(model_path.name + ".json")

    if not model_path.is_file() or not config_path.is_file():
        raise RuntimeError("Piper model or config file is missing")

    with config_path.open("r", encoding="utf-8") as source:
        config_dict = json.load(source)

    options = onnxruntime.SessionOptions()
    options.intra_op_num_threads = args.threads

    return PiperVoice(
        config=PiperConfig.from_dict(config_dict),
        session=onnxruntime.InferenceSession(
            os.fspath(model_path),
            sess_options=options,
            providers=["CPUExecutionProvider"],
        ),
        espeak_data_dir=Path(espeak_data_dir),
        download_dir=model_path.parent,
    ), model_path.name.removesuffix(".onnx")


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
            if 0 < len(operation_id.encode("utf-8")) <= MAX_OPERATION_ID_BYTES:
                return operation_id
        except UnicodeEncodeError:
            pass
    return None


def synthesize_pcm(voice, text: str):
    """Return (pcm_bytes, sample_rate), or None if the audio is unusable."""
    pcm = bytearray()
    sample_rate = None

    for chunk in voice.synthesize(text):
        if (
            chunk.sample_width != EXPECTED_SAMPLE_WIDTH_BYTES
            or chunk.sample_channels != EXPECTED_CHANNELS
            or (sample_rate is not None and chunk.sample_rate != sample_rate)
        ):
            return None
        sample_rate = chunk.sample_rate
        pcm += chunk.audio_int16_bytes

        # Stop early: an oversize answer is rejected anyway.
        if len(pcm) > MAX_WAV_BYTES:
            return None

    if sample_rate is None or not pcm:
        return None

    return bytes(pcm), int(sample_rate)


def write_wav(output_path: str, pcm: bytes, sample_rate: int) -> int:
    try:
        with wave.open(output_path, "wb") as target:
            target.setnchannels(EXPECTED_CHANNELS)
            target.setsampwidth(EXPECTED_SAMPLE_WIDTH_BYTES)
            target.setframerate(sample_rate)
            target.writeframes(pcm)
        return os.path.getsize(output_path)
    except BaseException:
        with contextlib.suppress(OSError):
            os.remove(output_path)
        raise


def handle_request(voice, request: dict) -> dict:
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

    synthesized = synthesize_pcm(voice, text)

    if synthesized is None:
        return error_result(operation_id, "invalid_audio")

    pcm, sample_rate = synthesized
    num_samples = len(pcm) // EXPECTED_SAMPLE_WIDTH_BYTES

    if num_samples > MAX_AUDIO_SECONDS * sample_rate:
        return error_result(operation_id, "invalid_audio")

    wav_size = write_wav(output_path, pcm, sample_rate)

    if wav_size > MAX_WAV_BYTES:
        os.remove(output_path)
        return error_result(operation_id, "invalid_audio")

    return {
        "type": "completed",
        "protocol": PROTOCOL_VERSION,
        "id": operation_id,
        "sample_rate": sample_rate,
        "num_samples": num_samples,
        "wav_bytes": wav_size,
    }


def ready_frame(voice, voice_name: str, threads: int) -> dict:
    return {
        "type": "ready",
        "protocol": PROTOCOL_VERSION,
        "pid": os.getpid(),
        "engine": ENGINE_NAME,
        "precision": PRECISION_NAME,
        "voice": voice_name,
        "language": "en",
        "provider": "cpu",
        "threads": threads,
        "sample_rate": voice.config.sample_rate,
        "num_speakers": voice.config.num_speakers,
    }


def serve(voice, reader) -> int:
    while True:
        try:
            line = read_request_line(reader)
        except WorkerFrameError as error:
            emit_frame(error_result(None, error.category))
            return 1

        if line is None:
            return 0

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
                    result = handle_request(voice, request)
        except json.JSONDecodeError:
            result = error_result(None, "invalid_json")
        except Exception as error:
            print(f"piper worker: {type(error).__name__}", file=sys.stderr, flush=True)
            result = error_result(operation_id, "worker_error")

        emit_frame(result)


def main() -> int:
    args = parse_args()

    with redirect_process_stdout_to_stderr():
        runtime = load_runtime()
        voice, voice_name = load_voice(args, runtime)

    emit_frame(ready_frame(voice, voice_name, args.threads))

    return serve(voice, sys.stdin.buffer)


if __name__ == "__main__":
    raise SystemExit(main())
