#!/usr/bin/env python3

import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
import wave
from pathlib import Path
from types import SimpleNamespace


WORKER_PATH = (
    Path(__file__).resolve().parents[5]
    / "scripts"
    / "tts"
    / "piper_worker.py"
)
SPEC = importlib.util.spec_from_file_location("jarvis_piper_worker", WORKER_PATH)
worker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(worker)

PIPER_SAMPLE_RATE = 22050


class FakeVoice:
    """Stands in for PiperVoice: yields one AudioChunk-shaped object per sentence."""

    def __init__(self, chunks=None, sample_rate=PIPER_SAMPLE_RATE):
        self.config = SimpleNamespace(sample_rate=sample_rate, num_speakers=1)
        self.chunks = chunks if chunks is not None else [self.chunk(1000)]
        self.texts = []

    @staticmethod
    def chunk(samples, rate=PIPER_SAMPLE_RATE, width=2, channels=1):
        return SimpleNamespace(
            sample_rate=rate,
            sample_width=width,
            sample_channels=channels,
            audio_int16_bytes=b"\x01\x00" * samples,
        )

    def synthesize(self, text):
        self.texts.append(text)
        yield from self.chunks


def speak(**overrides):
    request = {
        "type": "speak",
        "protocol": worker.PROTOCOL_VERSION,
        "id": "op-1",
        "text": "Hello there.",
        "output_path": "/tmp/unused.wav",
    }
    request.update(overrides)
    return request


class PiperWorkerTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.root = Path(self.temp_dir.name)

    def wav_path(self, name="out.wav"):
        return str(self.root / name)

    def test_ready_frame_declares_the_native_sample_rate(self):
        frame = worker.ready_frame(FakeVoice(), "en_GB-test-medium", 4)

        self.assertEqual(frame["type"], "ready")
        self.assertEqual(frame["protocol"], worker.PROTOCOL_VERSION)
        self.assertEqual(frame["pid"], os.getpid())
        self.assertEqual(frame["sample_rate"], PIPER_SAMPLE_RATE)
        self.assertEqual(frame["num_speakers"], 1)
        self.assertEqual(frame["threads"], 4)
        self.assertEqual(frame["voice"], "en_GB-test-medium")
        self.assertEqual(frame["engine"], "piper")
        self.assertEqual(
            set(frame),
            {
                "type", "protocol", "pid", "engine", "precision", "voice",
                "language", "provider", "threads", "sample_rate", "num_speakers",
            },
        )

    def test_successful_speak_writes_a_mono_16_bit_wav_at_22050_hz(self):
        voice = FakeVoice([FakeVoice.chunk(1000), FakeVoice.chunk(500)])
        path = self.wav_path()

        result = worker.handle_request(voice, speak(output_path=path))

        self.assertEqual(
            result,
            {
                "type": "completed",
                "protocol": worker.PROTOCOL_VERSION,
                "id": "op-1",
                "sample_rate": PIPER_SAMPLE_RATE,
                "num_samples": 1500,
                "wav_bytes": os.path.getsize(path),
            },
        )
        with wave.open(path, "rb") as wav:
            self.assertEqual(wav.getnchannels(), 1)
            self.assertEqual(wav.getsampwidth(), 2)
            self.assertEqual(wav.getframerate(), PIPER_SAMPLE_RATE)
            self.assertEqual(wav.getnframes(), 1500)
        self.assertEqual(voice.texts, ["Hello there."])

    def test_empty_text_is_rejected_without_synthesis(self):
        for text in ["", "   \n\t", None, 7]:
            with self.subTest(text=text):
                voice = FakeVoice()
                result = worker.handle_request(voice, speak(text=text))
                self.assertEqual(result["type"], "error")
                self.assertEqual(result["error"], "invalid_text")
                self.assertEqual(result["id"], "op-1")
                self.assertEqual(voice.texts, [])

    def test_limits_match_the_rust_side_and_the_supertonic_worker(self):
        # Pinned literally: the boundary tests below are relative to these
        # constants, so only this test catches a drifted limit.
        self.assertEqual(worker.MAX_TEXT_BYTES, 4096)
        self.assertEqual(worker.MAX_REQUEST_BYTES, 32 * 1024)
        self.assertEqual(worker.MAX_RESPONSE_BYTES, 4 * 1024)
        self.assertEqual(worker.MAX_OPERATION_ID_BYTES, 128)
        self.assertEqual(worker.MAX_WAV_BYTES, 16 * 1024 * 1024)
        self.assertEqual(worker.MAX_AUDIO_SECONDS, 120)

    def test_text_limit_accepts_the_last_byte_and_rejects_the_next(self):
        path = self.wav_path()
        accepted = worker.handle_request(
            FakeVoice(), speak(text="a" * worker.MAX_TEXT_BYTES, output_path=path)
        )
        self.assertEqual(accepted["type"], "completed")

        voice = FakeVoice()
        rejected = worker.handle_request(
            voice, speak(text="a" * (worker.MAX_TEXT_BYTES + 1), output_path=path)
        )
        self.assertEqual(rejected["error"], "text_too_large")
        self.assertEqual(voice.texts, [])

        # The limit counts UTF-8 bytes, not characters.
        multibyte = worker.handle_request(
            FakeVoice(), speak(text="é" * (worker.MAX_TEXT_BYTES // 2 + 1))
        )
        self.assertEqual(multibyte["error"], "text_too_large")

    def test_malformed_requests_get_fixed_error_codes(self):
        voice = FakeVoice()
        cases = [
            (speak(protocol=2), "unsupported_protocol"),
            (speak(type="play"), "unsupported_request"),
            (speak(id=None), "invalid_id"),
            (speak(id=""), "invalid_id"),
            (speak(id="x" * (worker.MAX_OPERATION_ID_BYTES + 1)), "invalid_id"),
        ]
        for request, code in cases:
            with self.subTest(code=code, request=request):
                self.assertEqual(worker.handle_request(voice, request)["error"], code)
        self.assertEqual(voice.texts, [])

        self.assertIsNone(
            worker.handle_request(voice, speak(id=None))["id"],
        )
        accepted = worker.handle_request(
            voice,
            speak(id="x" * worker.MAX_OPERATION_ID_BYTES, output_path=self.wav_path()),
        )
        self.assertEqual(accepted["type"], "completed")

    def test_output_path_must_be_an_absolute_wav_path(self):
        for path in [None, "", "relative.wav", "/tmp/out.mp3", "/tmp/out", 5]:
            with self.subTest(path=path):
                voice = FakeVoice()
                result = worker.handle_request(voice, speak(output_path=path))
                self.assertEqual(result["error"], "invalid_output_path")
                self.assertEqual(voice.texts, [])

    def test_only_the_requested_path_is_written(self):
        path = self.wav_path("answer.wav")
        worker.handle_request(FakeVoice(), speak(output_path=path))
        self.assertEqual(os.listdir(self.root), ["answer.wav"])

    def test_unusable_audio_is_rejected_and_writes_nothing(self):
        cases = {
            "no chunks": [],
            "empty chunk": [FakeVoice.chunk(0)],
            "stereo": [FakeVoice.chunk(10, channels=2)],
            "8-bit": [FakeVoice.chunk(10, width=1)],
            "mixed rates": [FakeVoice.chunk(10), FakeVoice.chunk(10, rate=16000)],
        }
        for name, chunks in cases.items():
            with self.subTest(name):
                path = self.wav_path(f"{name.replace(' ', '_')}.wav")
                result = worker.handle_request(
                    FakeVoice(chunks), speak(output_path=path)
                )
                self.assertEqual(result["error"], "invalid_audio")
                self.assertFalse(os.path.exists(path))

    def test_audio_longer_than_the_wav_limit_is_rejected(self):
        path = self.wav_path()
        too_big = [FakeVoice.chunk(worker.MAX_WAV_BYTES // 2 + 1)]
        result = worker.handle_request(FakeVoice(too_big), speak(output_path=path))
        self.assertEqual(result["error"], "invalid_audio")
        self.assertFalse(os.path.exists(path))

    def test_request_frames_are_bounded_and_utf8_errors_are_fixed(self):
        self.assertEqual(worker.read_request_line(io.BytesIO(b"{}\n")), "{}")
        self.assertIsNone(worker.read_request_line(io.BytesIO(b"")))

        with self.assertRaises(worker.WorkerFrameError) as oversized:
            worker.read_request_line(
                io.BytesIO(b"x" * (worker.MAX_REQUEST_BYTES + 1))
            )
        self.assertEqual(oversized.exception.category, "request_too_large")

        with self.assertRaises(worker.WorkerFrameError) as unterminated:
            worker.read_request_line(io.BytesIO(b"{}"))
        self.assertEqual(unterminated.exception.category, "invalid_request")

        with self.assertRaises(worker.WorkerFrameError) as invalid_utf8:
            worker.read_request_line(io.BytesIO(b"\xff\n"))
        self.assertEqual(invalid_utf8.exception.category, "invalid_utf8")
        self.assertNotIn("\\xff", str(invalid_utf8.exception))

    def run_serve(self, voice, stdin_bytes):
        """Drive the real request loop and capture the frames it writes."""
        frames = []
        original = worker.emit_frame
        worker.emit_frame = frames.append
        try:
            code = worker.serve(voice, io.BytesIO(stdin_bytes))
        finally:
            worker.emit_frame = original
        return code, frames

    def test_request_loop_answers_each_line_and_survives_bad_input(self):
        path = self.wav_path()
        good = json.dumps(speak(output_path=path)).encode() + b"\n"
        stdin = b"\n" + b"not json\n" + b"[1]\n" + good

        code, frames = self.run_serve(FakeVoice(), stdin)

        self.assertEqual(code, 0)
        self.assertEqual(
            [frame.get("error", frame["type"]) for frame in frames],
            ["invalid_json", "invalid_request", "completed"],
        )

    def test_synthesis_failure_becomes_worker_error_with_the_request_id(self):
        class Broken(FakeVoice):
            def synthesize(self, text):
                raise RuntimeError("secret detail")
                yield

        stdin = json.dumps(speak(output_path=self.wav_path())).encode() + b"\n"
        code, frames = self.run_serve(Broken(), stdin)

        self.assertEqual(code, 0)
        self.assertEqual(frames[0]["error"], "worker_error")
        self.assertEqual(frames[0]["id"], "op-1")
        self.assertNotIn("secret", json.dumps(frames))

    def test_oversize_request_line_ends_the_loop(self):
        code, frames = self.run_serve(
            FakeVoice(), b"x" * (worker.MAX_REQUEST_BYTES + 1)
        )
        self.assertEqual(code, 1)
        self.assertEqual(frames[0]["error"], "request_too_large")

    def test_native_and_python_stdout_chatter_stays_off_protocol_pipe(self):
        worker_path = str(WORKER_PATH)
        code = f"""
import importlib.util
import os
spec = importlib.util.spec_from_file_location("probe_worker", {worker_path!r})
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
with worker.redirect_process_stdout_to_stderr():
    print("python chatter")
    os.write(1, b"native chatter\\n")
os.write(1, b"ready\\n")
"""
        result = subprocess.run(
            [sys.executable, "-B", "-c", code],
            check=True,
            capture_output=True,
            text=False,
        )
        self.assertEqual(result.stdout, b"ready\n")
        self.assertIn(b"python chatter", result.stderr)
        self.assertIn(b"native chatter", result.stderr)

    def test_oversize_response_frame_is_replaced_by_a_fixed_error(self):
        code = f"""
import importlib.util
spec = importlib.util.spec_from_file_location("probe_worker", {str(WORKER_PATH)!r})
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
worker.emit_frame({{"type": "completed", "pad": "x" * worker.MAX_RESPONSE_BYTES}})
"""
        result = subprocess.run(
            [sys.executable, "-B", "-c", code], check=True, capture_output=True
        )
        frame = json.loads(result.stdout)
        self.assertEqual(frame["error"], "response_too_large")
        self.assertLessEqual(len(result.stdout), worker.MAX_RESPONSE_BYTES)


if __name__ == "__main__":
    unittest.main()
