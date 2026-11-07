#!/usr/bin/env bash
# Acceptance checks for jarvis-stt, run with curl against a base URL.
#
#   scripts/stt/acceptance.sh [BASE_URL] [CLIP_DIR]
#   scripts/stt/acceptance.sh http://jarvis-ai:8082/v1 /tmp/stt-spike
#
# CLIP_DIR needs clip1..clip5.wav, clip2-jieli.wav and clip6-long.wav (16 kHz mono 16-bit).
# Test files made from the clips (boundary lengths, a wrong rate) go in a temporary directory
# that is removed on exit. Needs bash, curl and python3. Prints transcripts, so run it on clips
# you are happy to see on screen. Exit status is 0 only when every check passed.
set -u

BASE=${1:-http://jarvis-ai:8082/v1}
CLIPS=${2:-/tmp/stt-spike}
ROOT=${BASE%/v1}
TRANSCRIBE="$BASE/audio/transcriptions"
CURL=(curl --noproxy '*' -s --connect-timeout 3)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
pass=0
fail=0

ok()   { pass=$((pass + 1)); printf 'PASS  %-34s %s\n' "$1" "$2"; }
bad()  { fail=$((fail + 1)); printf 'FAIL  %-34s %s\n' "$1" "$2"; }
check() { # name, condition result (0 = ok), detail
    if [ "$2" -eq 0 ]; then ok "$1" "$3"; else bad "$1" "$3"; fi
}

# post_file NAME FILE [extra curl args]: sets STATUS, SECONDS_TAKEN, BODY.
post_file() {
    local name=$1 file=$2
    shift 2
    local out
    out=$("${CURL[@]}" --max-time 40 -o "$WORK/body" -w '%{http_code} %{time_total}' \
        -F "file=@$file;type=audio/wav" "$@" "$TRANSCRIBE")
    STATUS=${out% *}
    SECONDS_TAKEN=${out#* }
    BODY=$(cat "$WORK/body")
}

is_error_body() { grep -q '"error":{"message":".*","type":"invalid_request_error"}' <<<"$BODY"; }

# --- test files derived from the clips ------------------------------------------------------
python3 -I - "$CLIPS" "$WORK" <<'PY'
import sys, wave
clips, work = sys.argv[1], sys.argv[2]
def read(name):
    with wave.open(f"{clips}/{name}.wav") as w:
        assert (w.getframerate(), w.getnchannels(), w.getsampwidth()) == (16000, 1, 2), name
        return w.readframes(w.getnframes())
def write(name, pcm, rate=16000, channels=1):
    with wave.open(f"{work}/{name}.wav", "wb") as w:
        w.setnchannels(channels); w.setsampwidth(2); w.setframerate(rate); w.writeframes(pcm)
short, long15 = read("clip1"), read("clip6-long")
def samples(pcm, n):  # first n samples, padded with the start of the clip if too short
    data = pcm
    while len(data) < 2 * n: data += pcm
    return data[:2 * n]
write("len-0.29s", samples(short, 4640))
write("len-0.30s", samples(short, 4800))
write("len-15.5s", samples(long15 + short, 248000))
write("len-15.6s", samples(long15 + short, 249600))
write("rate-8k", short, rate=8000)
write("stereo", short, channels=2)
PY
printf 'this is not audio\n' > "$WORK/not-a-wav.txt"
head -c 700000 /dev/zero > "$WORK/oversize.bin"
for needed in clip1 clip2 clip3 clip4 clip5 clip2-jieli clip6-long; do
    [ -f "$CLIPS/$needed.wav" ] || { echo "missing $CLIPS/$needed.wav"; exit 2; }
done

# --- health ----------------------------------------------------------------------------------
for path in "$ROOT/health" "$BASE/health"; do
    out=$("${CURL[@]}" --max-time 3 -w ' %{http_code}' "$path")
    check "GET ${path#http://}" $([ "$out" = '{"status":"ok"} 200' ] && echo 0 || echo 1) "$out"
done

# --- every clip transcribes -----------------------------------------------------------------
echo "--- clips (status, seconds, transcript) ---"
for clip in clip1 clip2 clip3 clip4 clip5 clip6-long; do
    post_file "$clip" "$CLIPS/$clip.wav"
    good=1
    [ "$STATUS" = 200 ] && [ "$BODY" != '{"text":""}' ] && [[ "$BODY" == '{"text":"'* ]] && good=0
    check "$clip" $good "status=$STATUS time=${SECONDS_TAKEN}s $BODY"
done
post_file clip2-jieli "$CLIPS/clip2-jieli.wav"
check "clip2-jieli returns empty text" $([ "$STATUS" = 200 ] && [ "$BODY" = '{"text":""}' ] && echo 0 || echo 1) \
    "status=$STATUS time=${SECONDS_TAKEN}s $BODY"

# --- rejections ------------------------------------------------------------------------------
echo "--- rejections (expect 4xx with the OpenAI error envelope) ---"
expect_4xx() { # name, expected status
    check "$1" $([ "$STATUS" = "$2" ] && is_error_body && echo 0 || echo 1) "status=$STATUS (want $2) $BODY"
}

start=$(date +%s.%N)
out=$("${CURL[@]}" --max-time 20 -o "$WORK/body" -w '%{http_code}' -X POST "$TRANSCRIBE")
STATUS=$out BODY=$(cat "$WORK/body")
elapsed=$(python3 -I -c "import sys; print(f'{float(sys.argv[2]) - float(sys.argv[1]):.2f}')" "$start" "$(date +%s.%N)")
expect_4xx "POST with no body" 400
check "no-body POST returns quickly" $(python3 -I -c "import sys; print(0 if float(sys.argv[1]) < 2 else 1)" "$elapsed") "${elapsed}s"

out=$("${CURL[@]}" --max-time 20 -o "$WORK/body" -w '%{http_code}' -F model=whisper-1 "$TRANSCRIBE")
STATUS=$out BODY=$(cat "$WORK/body")
expect_4xx "no file part" 400

post_file x "$WORK/not-a-wav.txt";           expect_4xx "non-WAV file" 400
post_file x "$WORK/rate-8k.wav";             expect_4xx "8 kHz WAV" 400
post_file x "$WORK/stereo.wav";              expect_4xx "stereo WAV" 400
post_file x "$WORK/len-0.29s.wav";           expect_4xx "0.29 s WAV (too short)" 400
post_file x "$WORK/len-15.6s.wav";           expect_4xx "15.6 s WAV (too long)" 400
post_file x "$CLIPS/clip1.wav" -F language=fr; expect_4xx "language=fr" 400
post_file x "$WORK/oversize.bin";            expect_4xx "700 KB body (over the byte limit)" 413

# A request that announces a body and never sends it must be dropped by the read timeout.
start=$(date +%s.%N)
out=$("${CURL[@]}" --max-time 30 -o "$WORK/body" -w '%{http_code}' -X POST \
    -H 'Content-Type: multipart/form-data; boundary=x' -H 'Content-Length: 1000' "$TRANSCRIBE")
rc=$?
elapsed=$(python3 -I -c "import sys; print(f'{float(sys.argv[2]) - float(sys.argv[1]):.2f}')" "$start" "$(date +%s.%N)")
check "announced body never sent" $(python3 -I -c "import sys; print(0 if float(sys.argv[1]) < 20 else 1)" "$elapsed") \
    "curl exit=$rc status=$out gave up after ${elapsed}s (read timeout, not a hang)"

# --- boundaries that must be accepted ---------------------------------------------------------
echo "--- accepted at the boundary ---"
post_file x "$WORK/len-0.30s.wav"
check "0.30 s WAV accepted" $([ "$STATUS" = 200 ] && echo 0 || echo 1) "status=$STATUS time=${SECONDS_TAKEN}s $BODY"
post_file x "$WORK/len-15.5s.wav"
check "15.5 s WAV accepted" $([ "$STATUS" = 200 ] && echo 0 || echo 1) "status=$STATUS time=${SECONDS_TAKEN}s $BODY"
post_file x "$CLIPS/clip3.wav" -F language=en -F model=whisper-1 -F prompt=ignored -F temperature=0
check "language=en and ignored fields" $([ "$STATUS" = 200 ] && echo 0 || echo 1) "status=$STATUS $BODY"

# --- health stays responsive during a long transcription ---------------------------------------
echo "--- health during a transcription ---"
( "${CURL[@]}" --max-time 40 -o "$WORK/long-body" -w '%{time_total}' \
    -F "file=@$CLIPS/clip6-long.wav;type=audio/wav" "$TRANSCRIBE" > "$WORK/long-time" ) &
long_pid=$!
sleep 1
polls=0 slow=0 worst=0
while kill -0 "$long_pid" 2> /dev/null; do
    t=$("${CURL[@]}" --max-time 3 -o /dev/null -w '%{time_total}' "$ROOT/health")
    polls=$((polls + 1))
    [ "$(python3 -I -c "import sys; print(1 if float(sys.argv[1]) > 1 else 0)" "$t")" = 1 ] && slow=$((slow + 1))
    worst=$t
    sleep 0.5
done
wait "$long_pid"
check "health answers while transcribing" $([ "$polls" -ge 3 ] && [ "$slow" -eq 0 ] && echo 0 || echo 1) \
    "$polls polls, $slow slower than 1 s, last ${worst}s; transcription took $(cat "$WORK/long-time")s"

# --- two transcriptions at once ----------------------------------------------------------------
echo "--- two at once (served one after the other) ---"
for n in 1 2; do
    ( "${CURL[@]}" --max-time 60 -o "$WORK/par-$n" -w '%{http_code} %{time_total}' \
        -F "file=@$CLIPS/clip5.wav;type=audio/wav" "$TRANSCRIBE" > "$WORK/par-$n.meta" ) &
done
wait
a=$(cat "$WORK/par-1.meta") b=$(cat "$WORK/par-2.meta")
check "two parallel requests both succeed" $([ "${a%% *}" = 200 ] && [ "${b%% *}" = 200 ] && echo 0 || echo 1) \
    "first: $a, second: $b"

echo "--- $pass passed, $fail failed ---"
[ "$fail" -eq 0 ]
