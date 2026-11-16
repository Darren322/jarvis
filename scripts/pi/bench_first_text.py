"""Time Jarvis's wait for the first word on the Pi, without touching Darren's data.

Run on the 8 GB Pi from the repo root (stdlib only):

    python3 scripts/pi/bench_first_text.py <jarvis binary> [--empty] [--pause=SECONDS] [prompts.txt]

It copies data/jarvis.sqlite3 to /tmp (or starts from an empty database with
--empty), starts the binary with speech, voice input and the orb port (8767, so the
screen does not connect to it) pointed away from the real ones, types the prompts and prints each answer with the
model's first_text, input and cached_input. The copy is deleted afterwards.
Prompts are one per line; the default five are the 2026-11-14 baseline.
Without --pause the prompts are typed all at once and run back to back. With it,
each prompt is typed SECONDS after the one before, which leaves Jarvis idle
between answers, as in a real conversation (a cache warm-up needs that time).
"""
import os, re, shutil, sqlite3, subprocess, sys, time

DEFAULT_PROMPTS = ["Hello", "How's the weather?", "Who are you?", "What time is it?", "Thanks, that is all"]

args = [a for a in sys.argv[1:] if a != "--empty" and not a.startswith("--pause=")]
empty = "--empty" in sys.argv[1:]
pause = next((float(a.split("=", 1)[1]) for a in sys.argv[1:] if a.startswith("--pause=")), None)
if not args:
    sys.exit(__doc__)
binary = args[0]
prompts = [l.strip() for l in open(args[1]) if l.strip()] if len(args) > 1 else DEFAULT_PROMPTS

scratch = "/tmp/jarvis-bench"
shutil.rmtree(scratch, ignore_errors=True)
os.makedirs(scratch)
database = scratch + "/db.sqlite3"
if not empty:
    source = sqlite3.connect("file:data/jarvis.sqlite3?mode=ro", uri=True)
    copy = sqlite3.connect(database)
    source.backup(copy)
    copy.close()
    source.close()
env = dict(os.environ, JARVIS_ARCHIVE_PATH=database, JARVIS_ORB_PORT="8767",
           JARVIS_TTS_ENGINE="", JARVIS_STT_BASE_URL="", JARVIS_STT_MODEL="", JARVIS_CAPTURE_DEVICE="")
jarvis = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
time.sleep(8)  # let the embedder connect, so a search is not keyword-only
if pause is None:
    jarvis.stdin.write("\n".join(prompts) + "\n")
else:
    for prompt in prompts:
        jarvis.stdin.write(prompt + "\n")
        jarvis.stdin.flush()
        time.sleep(pause)
jarvis.stdin.write("/exit\n")
jarvis.stdin.flush()
output, _ = jarvis.communicate(timeout=600)

total, turns = 0.0, 0
for line in output.splitlines():
    if line.startswith("Jarvis:"):
        print("ANSWER:", line[8:100])
    stage = re.search(r"turn=(\d+) .*first_text=([0-9.]+)(ms|s) .*input=(\d+) .*cached_input=(\d+)", line)
    if stage:
        turn, value, unit, tokens, cached = stage.groups()
        seconds = float(value) / (1000 if unit == "ms" else 1)
        total += seconds
        turns += 1
        print(f"  turn={turn} first_text={seconds:.2f}s input={tokens} cached={cached} new={int(tokens) - int(cached)}")
print(f"total first_text={total:.2f}s over {turns} model turns, {len(prompts)} prompts")
shutil.rmtree(scratch, ignore_errors=True)
