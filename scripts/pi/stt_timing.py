"""Time the speech-to-text server on the 2 s test clip, three times.

Run on the 8 GB Pi from the repo root: python3 scripts/pi/stt_timing.py
It reads the server settings from .env and prints only non-secret JARVIS_ settings.
"""
import io, json, os, time, urllib.request, wave, uuid
env = {}
for line in open(".env"):
    line = line.strip()
    if "=" in line and not line.startswith("#"):
        k, v = line.split("=", 1); env[k] = v.strip().strip('"')
print("settings:", {k: v for k, v in env.items() if k.startswith("JARVIS_") and not any(s in k for s in ("PASS", "KEY", "TOKEN"))})
raw = open("tests/unit/features/listening/fixtures/speech_16k_mono_s16le.raw", "rb").read()
buf = io.BytesIO()
with wave.open(buf, "wb") as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000); w.writeframes(raw)
wav = buf.getvalue()
def stt():
    b = uuid.uuid4().hex
    body = (f"--{b}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{env['JARVIS_STT_MODEL']}\r\n"
            f"--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\n").encode() + wav + f"\r\n--{b}--\r\n".encode()
    req = urllib.request.Request(env["JARVIS_STT_BASE_URL"] + "/audio/transcriptions", data=body, headers={"Content-Type": f"multipart/form-data; boundary={b}"})
    t = time.time(); out = urllib.request.urlopen(req, timeout=120).read(); return time.time() - t, out[:120]
for i in range(3):
    try:
        d, out = stt(); print("stt 2 s clip: %.2f s" % d, out)
    except Exception as e:
        print("stt failed:", e)
