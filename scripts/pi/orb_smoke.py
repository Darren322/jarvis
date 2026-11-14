"""Smoke-test the orb socket against the real Jarvis on the 8 GB Pi.

Run on the Pi from the repo root with XDG_RUNTIME_DIR set and no Jarvis running:

    python3 scripts/pi/orb_smoke.py

It starts ./target/release/jarvis, connects a fake orb, taps once to start
listening, reads states and mic levels for 3 s, cancels with /stop and exits.
Nothing is sent to speech-to-text or the model, so the archive is untouched.
"""
import socket, subprocess, time, os, stat, json, sys
path = os.environ["XDG_RUNTIME_DIR"] + "/jarvis-orb.sock"
jarvis = subprocess.Popen(["./target/release/jarvis"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
for _ in range(100):
    if os.path.exists(path): break
    time.sleep(0.1)
print("socket exists:", os.path.exists(path), "mode:", oct(stat.S_IMODE(os.stat(path).st_mode)) if os.path.exists(path) else None)
s = socket.socket(socket.AF_UNIX); s.connect(path); s.settimeout(0.2)
buf = b""; events = []
def pump(seconds):
    global buf
    end = time.time() + seconds
    while time.time() < end:
        try:
            data = s.recv(4096)
            if not data: events.append((time.time(), "CLOSED")); return
            buf += data
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                events.append((time.time(), json.loads(line)))
        except socket.timeout:
            pass
pump(1.0)
s.sendall(b'{"type":"tap"}\n')
pump(3.0)
jarvis.stdin.write("/stop\n"); jarvis.stdin.flush()
pump(1.5)
jarvis.stdin.write("/exit\n"); jarvis.stdin.flush()
pump(1.0)
out, _ = jarvis.communicate(timeout=20)
states = [(round(t - events[0][0], 2), e["state"]) for t, e in events if isinstance(e, dict) and e["type"] == "state"]
levels = [(t, e) for t, e in events if isinstance(e, dict) and e["type"] == "level"]
print("states:", states)
print("levels:", len(levels), "sources:", sorted({e["source"] for _, e in levels}))
if len(levels) > 2:
    span = levels[-1][0] - levels[0][0]
    vals = [e["level"] for _, e in levels]
    print("level rate per s: %.1f  min %.4f max %.4f" % ((len(levels) - 1) / span, min(vals), max(vals)))
print("closed by server at exit:", any(e == "CLOSED" for _, e in events), " socket removed:", not os.path.exists(path))
print("--- jarvis output ---"); print(out[-900:])
