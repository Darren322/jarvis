#!/usr/bin/env python3

import hashlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace


WORKER_PATH = (
    Path(__file__).resolve().parents[3]
    / "scripts"
    / "tts"
    / "supertonic_worker.py"
)
SPEC = importlib.util.spec_from_file_location("jarvis_supertonic_worker", WORKER_PATH)
worker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(worker)


class SupertonicWorkerTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.root = Path(self.temp_dir.name)
        self.script_dir = self.root / "scripts"
        self.script_dir.mkdir()
        self.model_dir = self.root / "model"
        self.model_dir.mkdir()
        (self.script_dir / "supertonic_worker.py").touch()
        self.original_worker_path = worker.__file__
        worker.__file__ = str(self.script_dir / "supertonic_worker.py")
        self.addCleanup(setattr, worker, "__file__", self.original_worker_path)

    def make_bundle(self):
        model_files = {}
        for filename in worker.EXPECTED_MODEL_FILES:
            contents = f"fixture:{filename}".encode("utf-8")
            (self.model_dir / filename).write_bytes(contents)
            model_files[filename] = {
                "sha256": hashlib.sha256(contents).hexdigest(),
            }

        voice_contents = b"fixture:verified-M5"
        (self.model_dir / "voice.bin").write_bytes(voice_contents)
        voice_hash = hashlib.sha256(voice_contents).hexdigest()
        lock = {
            "schema_version": 1,
            "supertonic": {
                "model": "supertonic-3",
                "precision": "int8",
                "package_id": "fixture-package",
                "files": model_files,
            },
            "voice": {
                "name": "M5",
                "sid": 0,
                "generated": {
                    "file": "voice.bin",
                    "sha256": voice_hash,
                },
            },
        }
        manifest = {
            "schema_version": 1,
            "engine": "supertonic-3",
            "precision": "int8",
            "package_id": "fixture-package",
            "voice": "M5",
            "sid": 0,
            "files": {
                **model_files,
                "voice.bin": {"sha256": voice_hash},
            },
        }
        (self.script_dir / "assets.lock.json").write_text(
            json.dumps(lock), encoding="utf-8"
        )
        (self.model_dir / "manifest.json").write_text(
            json.dumps(manifest), encoding="utf-8"
        )
        return SimpleNamespace(
            model_dir=self.model_dir,
            voice_style=self.model_dir / "voice.bin",
        ), manifest

    def test_bundle_verification_uses_locked_files_manifest_and_m5(self):
        args, manifest = self.make_bundle()
        self.assertEqual(worker.verify_bundle(args)["voice"], "M5")

        manifest["sid"] = 1
        (self.model_dir / "manifest.json").write_text(
            json.dumps(manifest), encoding="utf-8"
        )
        with self.assertRaises(RuntimeError):
            worker.verify_bundle(args)

        manifest["sid"] = 0
        (self.model_dir / "manifest.json").write_text(
            json.dumps(manifest), encoding="utf-8"
        )
        (self.model_dir / "voice.bin").write_bytes(b"changed voice")
        with self.assertRaises(RuntimeError):
            worker.verify_bundle(args)

    def test_request_frames_are_bounded_and_utf8_errors_are_fixed(self):
        self.assertEqual(worker.read_request_line(io.BytesIO(b"{}\n")), "{}")

        with self.assertRaises(worker.WorkerFrameError) as oversized:
            worker.read_request_line(
                io.BytesIO(b"x" * (worker.MAX_REQUEST_BYTES + 1))
            )
        self.assertEqual(oversized.exception.category, "request_too_large")

        with self.assertRaises(worker.WorkerFrameError) as invalid_utf8:
            worker.read_request_line(io.BytesIO(b"\xff\n"))
        self.assertEqual(invalid_utf8.exception.category, "invalid_utf8")
        self.assertNotIn("\\xff", str(invalid_utf8.exception))

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


if __name__ == "__main__":
    unittest.main()
