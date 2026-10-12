# Manual BGE GGUF deployment for the two-Pi Jarvis setup

Prepared 2026-10-09; reviewed 2026-10-10. Deployment is **not verified**. Codex has no SSH access and has run none of these Pi commands. This guide changes only the separate embedding deployment; Jarvis's memory integration waits for your results.

## What will run where

| Host | Responsibility |
| --- | --- |
| **16 GB Pi — jarvis-ai.local** | Existing Gemma llama-server on 8080; new BGE llama-server on suggested port 8081. You download and install the embedding model here. |
| **8 GB Pi — Jarvis** | Rust/Tokio application, TTS, and SQLite (archive, memories, FTS5 and sqlite-vec indexes). For now only test LAN access from here. Do not change Jarvis configuration or remove ONNX assets yet. |

Run the shell blocks below in **Bash** on the indicated Pi. Keep Gemma idle during embedding measurements. Do not run Jarvis at the same time as these manual inference tests: its idle embedding pass and chat requests would skew the measurements.

## 1. Model and recipe

The current repository uses `Qdrant/bge-small-en-v1.5-onnx-Q` at revision `aa8f8b060edb00e03bfdd08813a2949946c8ba55`, through FastEmbed 4.9.1/native ONNX Runtime 1.20.1. The source model family is **BAAI/bge-small-en-v1.5**, not BGE-base, BGE-large, BGE-M3 or a BGE reranker.

Use this initial GGUF candidate:

| Property | Value |
| --- | --- |
| Publisher | `CompendiumLabs/bge-small-en-v1.5-gguf` (third-party conversion) |
| Immutable repository revision | `d32f8c040ea3b516330eeb75b72bcc2d3a780ab7` |
| File | `bge-small-en-v1.5-q8_0.gguf` |
| Size | 36,806,944 bytes |
| SHA-256 | `ec38e8da142596baa913124ae50550de284b6916bf59577ef2f0cb9660c2f514` |
| Expected dimensions / maximum sequence | 384 / 512 tokens, including special tokens and query instruction |
| Pooling / output | CLS / L2-normalized float vector |
| Retrieval query input | `Represent this sentence for searching relevant passages: ` followed by the query, exactly once |
| Document input | Raw passage; no prefix |

Q8_0 is a small initial CPU candidate, not a claim that it is faster or more accurate than F16 or the current ONNX model. The publisher identifies the BAAI model but does not pin the original conversion's upstream checkpoint revision. The GGUF artifact itself is pinned and hash-verifiable. Expected shape/recipe come from upstream; Pi startup metadata and HTTP tests must confirm actual behavior.

Sources: [publisher card](https://huggingface.co/CompendiumLabs/bge-small-en-v1.5-gguf/blob/main/README.md), [artifact and SHA-256](https://huggingface.co/CompendiumLabs/bge-small-en-v1.5-gguf/blob/d32f8c040ea3b516330eeb75b72bcc2d3a780ab7/bge-small-en-v1.5-q8_0.gguf), [BAAI model/config](https://huggingface.co/BAAI/bge-small-en-v1.5/blob/main/config.json), [CLS pooling config](https://huggingface.co/BAAI/bge-small-en-v1.5/blob/main/1_Pooling/config.json), [BAAI recipe](https://huggingface.co/BAAI/bge-small-en-v1.5).

**Rebuild required later:** GGUF Q8_0 and ONNX INT8 are different vector recipes even though both return 384 values. Do not mix their vectors in the current index. Preserve SQLite; integration assigns a new model fingerprint, and Jarvis re-embeds every memory during idle time. Hybrid FTS5 + sqlite-vec fusion stays unchanged. See [the integration guide](BGE_SERVER_INTEGRATION.md).

## 2. First checkpoint — 16 GB Pi only

Collect the binary identity before running the launch recipe. These are read-only checks:

```bash
uname -m
getconf LONG_BIT
cat /etc/os-release
free -h
command -v llama-server || true
pgrep -x llama-server || true
sudo ss -ltnp '( sport = :8080 or sport = :8081 )'
```

Expect a 64-bit ARM installation (`aarch64`, `64`). If llama-server is not in PATH, find the binary used by the running Gemma process without changing it:

```bash
for pid in $(pgrep -x llama-server); do
    printf 'PID %s binary: ' "$pid"
    sudo readlink -f "/proc/$pid/exe"
done
```

Set the actual absolute executable path printed above; this example is a placeholder, not an assumed installation location:

```bash
export LLAMA_SERVER='/replace/with/actual/path/to/llama-server'
test -x "$LLAMA_SERVER"
mkdir -p "$HOME/jarvis-embedding-check"
"$LLAMA_SERVER" --version 2>&1 | tee "$HOME/jarvis-embedding-check/llama-version.txt"
"$LLAMA_SERVER" --help > "$HOME/jarvis-embedding-check/llama-help.txt" 2>&1
sha256sum "$LLAMA_SERVER"
grep -E -- 'embedding|pooling|ctx-size|batch-size|ubatch-size|parallel|threads|gpu-layers|host|port|alias|version' \
    "$HOME/jarvis-embedding-check/llama-help.txt"
```

**Stop here and send the version and relevant help output to this chat.** This is an evidence checkpoint, not another architecture approval. Codex cannot inspect your installed binary. We will match its reported build to primary documentation before you execute the gated launch commands below. If this is a container-only executable or a deleted/replaced binary, report that; do not copy an executable out of the container and assume its shared libraries will work.

The recipe uses interfaces verified in [llama.cpp b5046's versioned server documentation](https://raw.githubusercontent.com/ggml-org/llama.cpp/b5046/examples/server/README.md) and [commit-pinned server implementation](https://raw.githubusercontent.com/ggml-org/llama.cpp/74d4f5b041ad837153b0e90fc864b8290e01d8d5/examples/server/server.cpp), with current upstream documentation cross-checked. **b5046 is a reference, not an instruction to downgrade, upgrade or replace Gemma's runtime.** If your build needs changes, select a separate embedding binary after inspecting its actual version.

## 3. Download and verify — 16 GB Pi only, after the checkpoint

Required tools: `curl`, `python3`, `sha256sum`; `sysstat` supplies `pidstat` for measurements. On Debian/Raspberry Pi OS, install missing tools if needed:

```bash
sudo apt-get update
sudo apt-get install -y curl python3 sysstat
```

Store the pinned model in a revision-specific directory. The command verifies the downloaded temporary file before moving it into its final name:

```bash
export BGE_DIR="$HOME/models/bge-small-en-v1.5-gguf/d32f8c040ea3b516330eeb75b72bcc2d3a780ab7"
export BGE_FILE='bge-small-en-v1.5-q8_0.gguf'
export BGE_SHA='ec38e8da142596baa913124ae50550de284b6916bf59577ef2f0cb9660c2f514'
mkdir -p "$BGE_DIR"
(
    set -euo pipefail
    cd "$BGE_DIR"
    curl --fail --location --retry 3 --connect-timeout 15 --max-time 600 \
      --output "$BGE_FILE.part" \
      'https://huggingface.co/CompendiumLabs/bge-small-en-v1.5-gguf/resolve/d32f8c040ea3b516330eeb75b72bcc2d3a780ab7/bge-small-en-v1.5-q8_0.gguf'
    printf '%s  %s\n' "$BGE_SHA" "$BGE_FILE.part" | sha256sum --check -
    test "$(stat -c %s "$BGE_FILE.part")" = 36806944
    mv "$BGE_FILE.part" "$BGE_FILE"
    printf '%s  %s\n' "$BGE_SHA" "$BGE_FILE" > SHA256SUMS
)
```

Asset download retries are harmless here; these are not inference retries. A hash mismatch is a stop condition. The 36.8 MB file size is not a RAM estimate.

## 4. Manual startup — 16 GB Pi only

First record Gemma's baseline while **no embedding server is running**, using section 6's chat probe. Then return here. Do not stop or restart Gemma.

Before launch, recheck the model against the pinned SHA-256 from section 3, then record the hash for your results. After the version checkpoint, run this in terminal A. It stays in the foreground; use another terminal for curl. Port 8081 is the proposed test port; if occupied, choose an unused one and consistently update the later commands.

~~~bash
printf '%s  %s\n' "$BGE_SHA" "$BGE_DIR/$BGE_FILE" | sha256sum --check -
sha256sum "$BGE_DIR/$BGE_FILE"
~~~

```bash
"$LLAMA_SERVER" \
  --model "$BGE_DIR/$BGE_FILE" \
  --alias bge-small-en-v1.5 \
  --embedding --pooling cls \
  --ctx-size 512 --batch-size 512 --ubatch-size 512 \
  --parallel 1 --threads 2 --threads-batch 2 \
  --gpu-layers 0 \
  --host 127.0.0.1 --port 8081 \
  2>&1 | tee "$HOME/jarvis-embedding-check/bge-startup.log"
```

Why these settings: one slot and one input at a time keep work small. Two CPU threads are a starting measurement candidate. `batch-size`/`ubatch-size` are **token capacities**, not document counts: a non-causal BERT input must fit in a physical batch, so do not reduce these to 1. Keep the query instruction and special tokens inside 512. CPU-only is explicit. No chat template or Gemma flags are applied to BGE. The OpenAI embeddings endpoint normalizes pooled output; the tests below verify the norm rather than relying only on configuration.

Capture startup metadata showing BERT architecture, 384 embedding width, 512 trained context and CLS pooling (some builds report pooling as enum value 2). Report any differing metadata. Do not infer these from the model filename alone.

In terminal B, check loading/readiness and model alias:

```bash
curl --noproxy '*' --max-time 5 -i http://127.0.0.1:8081/health
curl --noproxy '*' --fail --max-time 5 http://127.0.0.1:8081/v1/models
```

During loading, 503 is possible. Repeat manually after startup finishes; do not submit inference until health is 200. Health means ready to serve, **not idle**, and no startup duration has been measured until you record it. The health response and model alias indicate readiness/routing only; neither attests which GGUF file or SHA-256 is loaded.

## 5. Curl and vector validation — 16 GB Pi only

Use `/v1/embeddings`, not `/v1/chat/completions`. Query prefix is supplied explicitly; llama.cpp is not assumed to add it.

```bash
curl --noproxy '*' --fail-with-body --connect-timeout 3 --max-time 30 \
  http://127.0.0.1:8081/v1/embeddings \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"bge-small-en-v1.5","input":"Represent this sentence for searching relevant passages: Where did I leave my keys?","encoding_format":"float"}' \
  --output "$HOME/jarvis-embedding-check/query.json" \
  --write-out 'query HTTP=%{http_code} total_s=%{time_total}\n'

curl --noproxy '*' --fail-with-body --connect-timeout 3 --max-time 30 \
  http://127.0.0.1:8081/v1/embeddings \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"bge-small-en-v1.5","input":"The keys are in the blue bowl beside the front door.","encoding_format":"float"}' \
  --output "$HOME/jarvis-embedding-check/document.json" \
  --write-out 'document HTTP=%{http_code} total_s=%{time_total}\n'

python3 - <<'PY'
import json, math, pathlib
root = pathlib.Path.home() / 'jarvis-embedding-check'
for name in ('query', 'document'):
    data = json.loads((root / f'{name}.json').read_text())['data']
    assert len(data) == 1 and data[0]['index'] == 0, data
    v = data[0]['embedding']
    assert len(v) == 384, len(v)
    assert all(isinstance(x, (int, float)) and math.isfinite(x) for x in v)
    norm = math.sqrt(sum(x*x for x in v))
    assert abs(norm - 1.0) < 0.001, norm
    print(name, 'dimensions=', len(v), 'L2_norm=', norm)
PY
```

If a request times out, stop sending more inference until you establish completion or stop/restart **only the embedding server** and observe it exiting. Curl timing out does not terminate native computation. This manual pause keeps benchmark samples interpretable; it does not imply that Jarvis must reconcile remote server state after a timeout. A timeout means the client stopped waiting, not that the server canceled work. Do not add automatic inference retry flags.

## 6. Small acceptance measurements — 16 GB Pi

These commands deliberately generate a few non-sensitive test requests when **you** run them. No benchmark has been run by Codex. Save raw samples; a small fixture is not a general quality evaluation.

Create this standard-library-only probe (outside the Jarvis repository):

```bash
mkdir -p "$HOME/jarvis-embedding-check"
cat > "$HOME/jarvis-embedding-check/probe.py" <<'PY'
import json, math, os, statistics, sys, time, urllib.request

opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def request(url, payload=None):
    body = None if payload is None else json.dumps(payload).encode()
    return urllib.request.Request(url, data=body, headers={'Content-Type': 'application/json'})

def embed(text):
    start = time.monotonic()
    base = os.environ.get('EMBED_BASE', 'http://127.0.0.1:8081')
    with opener.open(request(base + '/v1/embeddings', {
        'model': 'bge-small-en-v1.5', 'input': text, 'encoding_format': 'float'
    }), timeout=30) as response:
        result = json.load(response)
    rows = result['data']
    assert len(rows) == 1 and rows[0]['index'] == 0
    v = rows[0]['embedding']
    assert len(v) == 384 and all(math.isfinite(x) for x in v)
    norm = math.sqrt(sum(x*x for x in v))
    assert abs(norm - 1) < 0.001, norm
    return v, time.monotonic() - start

def embeddings():
    query = 'Represent this sentence for searching relevant passages: Where did I leave my keys?'
    passage = 'The keys are in the blue bowl beside the front door.'
    # About a paragraph-sized input; token count still must be checked separately.
    longer = (passage + ' ') * 16
    q, cold = embed(query)
    good, _ = embed(passage)
    bad, _ = embed('Saturn is a planet surrounded by rings of ice and rock.')
    relevant = sum(a*b for a,b in zip(q, good))
    unrelated = sum(a*b for a,b in zip(q, bad))
    print(json.dumps({'first_request_s': cold, 'relevant_cosine': relevant,
                      'unrelated_cosine': unrelated}), flush=True)
    assert relevant > unrelated, 'Inspect model/pooling/recipe before integrating'
    for label, text in [('query', query), ('document', passage), ('longer_document', longer)]:
        samples = [embed(text)[1] for _ in range(5)]
        print(json.dumps({'kind': label, 'seconds': samples,
                          'median_s': statistics.median(samples), 'max_s': max(samples)}), flush=True)

def chat(label):
    base = os.environ.get('CHAT_BASE', 'http://127.0.0.1:8080')
    with opener.open(request(base + '/v1/models'), timeout=5) as response:
        models = json.load(response)['data']
    model = os.environ.get('CHAT_MODEL')
    if not model:
        assert len(models) == 1, 'Set CHAT_MODEL to the deployed model ID'
        model = models[0]['id']
    for i in range(3):
        start, first, done, finish = time.monotonic(), None, False, False
        payload = {'model': model, 'messages': [{'role': 'user', 'content':
                   'Briefly explain why leaves look green.'}],
                   'temperature': 0, 'max_tokens': 48, 'stream': True}
        with opener.open(request(base + '/v1/chat/completions', payload), timeout=90) as response:
            for line in response:
                if not line.startswith(b'data: '):
                    continue
                data = line[6:].strip()
                if data == b'[DONE]':
                    done = True
                    break
                chunk = json.loads(data)
                if 'error' in chunk:
                    raise RuntimeError(chunk['error'])
                for choice in chunk.get('choices', []):
                    if choice.get('finish_reason') is not None:
                        finish = True
                    if first is None and choice.get('delta', {}).get('content'):
                        first = time.monotonic() - start
        assert done or finish, 'Stream ended without completion evidence; stop and inspect'
        print(json.dumps({'label': label, 'sample': i + 1,
                          'first_content_s': first, 'total_s': time.monotonic() - start,
                          'model': model}), flush=True)

if sys.argv[1] == 'embed':
    embeddings()
elif sys.argv[1] == 'chat':
    chat(sys.argv[2])
else:
    raise SystemExit('Use: probe.py embed OR probe.py chat LABEL')
PY
```

Measure in this order, with no other inference clients active:

```bash
# Before section 4: Gemma running, embedding server not running.
python3 "$HOME/jarvis-embedding-check/probe.py" chat baseline \
  | tee "$HOME/jarvis-embedding-check/chat-baseline.jsonl"

# After sections 4–5: BGE loaded but idle. No embedding requests during this command.
python3 "$HOME/jarvis-embedding-check/probe.py" chat bge_loaded_idle \
  | tee "$HOME/jarvis-embedding-check/chat-bge-idle.jsonl"

# Gemma idle: embedding samples run sequentially, never concurrently with chat.
python3 "$HOME/jarvis-embedding-check/probe.py" embed \
  | tee "$HOME/jarvis-embedding-check/embedding-samples.jsonl"

# Only after embedding probe exits successfully: check chat responsiveness again.
python3 "$HOME/jarvis-embedding-check/probe.py" chat after_embedding \
  | tee "$HOME/jarvis-embedding-check/chat-after-embedding.jsonl"
```

`first_content_s` measures the first nonempty assistant content event, not HTTP headers or hidden reasoning. A null value needs inspection. Identical prompts may benefit from prompt caching; compare first and warm samples separately. The first embedding sample is first-request timing after any server warmup, not cold model load. Python urllib's `timeout=30` and `timeout=90` values limit individual blocking socket operations, not total wall-clock request duration; a stream can last longer across reads. These are manual probe limits, not Jarvis request deadlines. Integration will need a separate total request deadline, chosen from measurements. To measure startup separately, record monotonic elapsed time from process launch to the first health 200; don't call the embedding timing “startup time.”

In another terminal, monitor CPU, resident RAM and swap during the samples:

```bash
pgrep -x llama-server
ps -C llama-server -o pid,comm,rss,%cpu,etimes
free -h
vmstat 1 15
pidstat -u -r -C llama-server 1 30
command -v vcgencmd >/dev/null && vcgencmd measure_temp
command -v vcgencmd >/dev/null && vcgencmd get_throttled
```

Identify the embedding PID by port using `sudo ss -ltnp 'sport = :8081'`. `ps` RSS is KiB; its CPU percentage is a lifetime average, while pidstat gives interval samples. 100% CPU approximately represents one full core. RSS is not total system memory usage; report both process and host values. Thermal throttling, cooling and swap activity can explain slow samples. Do not infer peak memory from one snapshot.

Initial review criteria: all requests succeed; vectors have 384 finite values and norm within 0.001 of 1; relevant passage outranks the unrelated example; both models stay resident without OOM/restart or sustained swapping; compare Gemma baseline versus BGE-loaded-idle and after-embedding timings. Share measured values before we choose production timeout/latency budgets. This sequential procedure measures coexistence and basic responsiveness only; it does not test Jarvis foreground prioritization or the later bounded, non-preemptive embedding gate. A focused Jarvis check belongs after integration. Deliberate concurrent embedding/generation is excluded from this manual acceptance because you requested non-overlap.

## 7. Manual LAN verification and benchmark — both Pis

Complete sections 4–6 first. This step checks the same BGE process over the LAN before any permanent service is installed. These examples use port 8081; if section 4 used another free port, use that port consistently below. Keep Jarvis stopped during these manual inference tests; keep Gemma running, and do not run Gemma inference during the embedding samples.

On the 16 GB Pi, stop only the foreground loopback BGE process from terminal A with Ctrl-C. Confirm that its test port is closed; do not stop Gemma:

~~~bash
sudo ss -ltnp 'sport = :8081'
~~~

After stopping the server, run this address-selection block in terminal A so the exported values are available to the next launch command. Check hostname -I against ip -4 addr show; use the address assigned to this Pi, not a loopback, public, or 8 GB Pi address. The test server below has no API authentication and is reachable by any host allowed through the network. Use it only on a trusted private LAN, do not configure router port forwarding, and keep the host firewall enabled. If you cannot restrict access to the 8 GB Pi or trust the LAN, do not expose the endpoint.

~~~bash
hostname -I
ip -4 addr show
export EMBED_BIND='REPLACE_WITH_16_GB_PI_PRIVATE_IPV4'
export EMBED_PORT=8081
python3 - "$EMBED_BIND" <<'PY'
import ipaddress, sys
address = ipaddress.ip_address(sys.argv[1])
assert address.version == 4
assert any(address in ipaddress.ip_network(cidr) for cidr in (
    '10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16'
))
assert not address.is_loopback and not address.is_unspecified
print("Selected private IPv4:", address)
PY
~~~

In terminal A on the 16 GB Pi, start the same model with the same limits as section 4; only the bind address changes. Keep this in the foreground. Do not stop or restart Gemma:

~~~bash
"$LLAMA_SERVER" \
  --model "$BGE_DIR/$BGE_FILE" \
  --alias bge-small-en-v1.5 \
  --embedding --pooling cls \
  --ctx-size 512 --batch-size 512 --ubatch-size 512 \
  --parallel 1 --threads 2 --threads-batch 2 \
  --gpu-layers 0 \
  --host "$EMBED_BIND" --port "$EMBED_PORT" \
  2>&1 | tee "$HOME/jarvis-embedding-check/bge-startup-lan.log"
~~~

In another terminal on the 16 GB Pi, set the same bind address and port, then check readiness and the configured model alias. A 200 health response and the alias do not prove which GGUF file or SHA-256 is loaded; use the pinned-file hash check in section 4 and retain the startup log as deployment evidence.

~~~bash
export EMBED_BIND='REPLACE_WITH_16_GB_PI_PRIVATE_IPV4'
export EMBED_PORT=8081
curl --noproxy '*' --max-time 5 -i "http://$EMBED_BIND:$EMBED_PORT/health"
curl --noproxy '*' --fail --max-time 5 "http://$EMBED_BIND:$EMBED_PORT/v1/models"
~~~

On the 8 GB Pi, do not install the model or change .env. JARVIS_EMBEDDING_MODEL_DIR still names local ONNX model and tokenizer files, not a URL, and Jarvis has no HTTP embedding setting. EMBED_BASE configures only the standalone probe.py; it is not a Jarvis setting. For the later Jarvis change, see [BGE_SERVER_INTEGRATION.md](BGE_SERVER_INTEGRATION.md).

On the 8 GB Pi, with Jarvis still stopped, use the stable mDNS name if it resolves. If it does not, substitute the verified private IPv4 address for jarvis-ai.local; do not use a public endpoint. Set the same test port used on the 16 GB Pi:

~~~bash
export EMBED_PORT=8081
getent ahostsv4 jarvis-ai.local
curl --noproxy '*' --fail --connect-timeout 3 --max-time 5 \
  http://jarvis-ai.local:$EMBED_PORT/health
curl --noproxy '*' --fail --connect-timeout 3 --max-time 5 \
  http://jarvis-ai.local:$EMBED_PORT/v1/models
curl --noproxy '*' --fail-with-body --connect-timeout 3 --max-time 30 \
  http://jarvis-ai.local:$EMBED_PORT/v1/embeddings \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"bge-small-en-v1.5","input":"Represent this sentence for searching relevant passages: Where did I leave my keys?","encoding_format":"float"}' \
  --output /tmp/jarvis-bge-lan-query.json \
  --write-out 'LAN HTTP=%{http_code} total_s=%{time_total}\n'
python3 - <<'PY'
import json, math
with open('/tmp/jarvis-bge-lan-query.json') as f:
    rows = json.load(f)['data']
assert len(rows) == 1 and rows[0]['index'] == 0
v = rows[0]['embedding']
assert len(v) == 384 and all(math.isfinite(x) for x in v)
norm = math.sqrt(sum(x*x for x in v))
assert abs(norm - 1) < 0.001
print('LAN vector dimensions=', len(v), 'L2_norm=', norm)
PY
~~~

For repeated LAN timings, copy the same standalone probe.py from section 6 to the 8 GB Pi and run:

~~~bash
EMBED_BASE=http://jarvis-ai.local:$EMBED_PORT python3 /path/to/probe.py embed
~~~

Record the raw samples and compare them with the local results from section 6. If .local fails, retry with the verified private IPv4 to separate DNS from connectivity. Do not disable the firewall to make the test pass. When finished, stop the foreground BGE process with Ctrl-C and confirm port 8081 is closed. Send the local and LAN results for review before deciding whether to install the optional service in section 8.

## 8. Optional automatic startup — 16 GB Pi only

Use this only after sections 4–7 are complete and you have reviewed the local and LAN measurements. Permanent startup is optional; the manual deployment and benchmarks do not depend on systemd. Before installing the service, stop the foreground BGE process from terminal A with Ctrl-C and confirm port 8081 is no longer listening. Do not use `pkill llama-server`, which would also stop Gemma. The dedicated service runs under your existing unprivileged account and starts independently of Jarvis.

Choose the 16 GB Pi's actual LAN IPv4 address from `hostname -I`. Prefer a DHCP reservation so it remains stable. Binding only that address makes the service accessible to the 8 GB Pi without binding every interface. Keep this on your trusted LAN; no router port-forward. This recipe does not configure API authentication or alter your firewall. If your LAN needs authentication, stop before exposing the service and adapt the version-matched API-key configuration.

Run in the same Bash session as the earlier path variables, or set them again:

```bash
hostname -I
export EMBED_BIND='REPLACE_WITH_16_GB_PI_LAN_IPV4'
export EMBED_PORT=8081
export LLAMA_SERVER="$(readlink -f "$LLAMA_SERVER")"
export EMBED_USER="$(id -un)"
export EMBED_GROUP="$(id -gn)"
(
set -euo pipefail
test "$EMBED_USER" != root
test -x "$LLAMA_SERVER"
test -r "$BGE_DIR/$BGE_FILE"
python3 - <<'PY'
import ipaddress, os
addr = ipaddress.ip_address(os.environ['EMBED_BIND'])
assert addr.version == 4 and addr.is_private and not addr.is_loopback and not addr.is_unspecified
assert 1024 <= int(os.environ['EMBED_PORT']) <= 65535
for key in ('LLAMA_SERVER', 'BGE_DIR'):
    p = os.environ[key]
    assert p.startswith('/') and not any(c in p for c in '\n\r"%$\\'), (key, p)
PY

# Stop if this name already exists; inspect it instead of overwriting another service.
test ! -e /etc/systemd/system/jarvis-embeddings.service
cat > "$HOME/jarvis-embedding-check/jarvis-embeddings.service" <<EOF
[Unit]
Description=Jarvis BGE embeddings (llama.cpp)
Wants=network-online.target
After=network-online.target
StartLimitIntervalSec=60
StartLimitBurst=3

[Service]
Type=simple
User=$EMBED_USER
Group=$EMBED_GROUP
WorkingDirectory="$BGE_DIR"
ExecStart="$LLAMA_SERVER" --model "$BGE_DIR/$BGE_FILE" --alias bge-small-en-v1.5 --embedding --pooling cls --ctx-size 512 --batch-size 512 --ubatch-size 512 --parallel 1 --threads 2 --threads-batch 2 --gpu-layers 0 --host $EMBED_BIND --port $EMBED_PORT
Restart=on-failure
RestartSec=5
TimeoutStopSec=30
KillMode=control-group
NoNewPrivileges=true
PrivateTmp=true
UMask=0077

[Install]
WantedBy=multi-user.target
EOF

cat "$HOME/jarvis-embedding-check/jarvis-embeddings.service"
sudo systemd-analyze verify "$HOME/jarvis-embedding-check/jarvis-embeddings.service"
sudo install -m 0644 "$HOME/jarvis-embedding-check/jarvis-embeddings.service" /etc/systemd/system/jarvis-embeddings.service
sudo systemctl daemon-reload
sudo systemctl enable --now jarvis-embeddings.service
systemctl status jarvis-embeddings.service --no-pager
journalctl -u jarvis-embeddings.service -n 60 --no-pager
curl --noproxy '*' --fail --max-time 5 "http://$EMBED_BIND:$EMBED_PORT/health"
)
```

The subshell stops installation on a failed validation. A first health request can fail while the model is still loading; inspect the journal and repeat the health check after loading. Do not add arbitrary `LD_LIBRARY_PATH` values: if systemd cannot load a shared library, inspect the binary with `ldd "$LLAMA_SERVER"` and reproduce its verified runtime path. The service uses the actual resolved binary; future runtime upgrades change its identity and need revalidation. `systemctl active` does not mean model-ready; check health and one validated embedding after startup. To stop just this service: `sudo systemctl stop jarvis-embeddings.service`. To disable automatic startup: `sudo systemctl disable --now jarvis-embeddings.service`.

No reboot is needed just for this setup. Check `systemctl is-enabled jarvis-embeddings.service`; verify health again after your next planned reboot. Binding a fixed address may fail if that interface/address is not ready despite network-online; diagnose the network's wait-online service or address reservation rather than silently binding publicly.

## 9. Troubleshooting

| Symptom | Next check |
| --- | --- |
| Unknown option or unsupported BERT/GGUF | Send actual `--version`, relevant `--help`, and error. Do not remove pooling/embedding flags until it starts accidentally; do not replace Gemma's working binary. |
| Hash/size mismatch | Do not launch. Remove or redownload only the `.part` file from the pinned URL; inspect HTTP failure/disk space. |
| Address already in use | `sudo ss -ltnp 'sport = :8081'`; stop only your manual embedding process, or choose a free port consistently. |
| HTTP 503 | Model may still be loading; inspect the foreground server output, or the service journal if using section 8. Health 503 is not an invitation to replay timed-out inference. |
| HTTP 400 / input too long / ubatch error | Keep the full sequence under 512 including special tokens/prefix; keep physical batch capacity 512 for this single-input setup. Do not enlarge context beyond the trained limit as a workaround. |
| Wrong dimensions, nonfinite values, or norm not near 1 | Check exact file/hash, CLS, `/v1/embeddings`, float encoding and build. Stop before indexing. |
| LAN connection refused | Check service bind address/port and health on the 16 GB Pi. Loopback startup intentionally cannot be reached from the 8 GB Pi. |
| `.local` resolution fails | Inspect `getent`/mDNS support and compare with the Pi's private IPv4. A private-IP curl test isolates DNS from HTTP; no public endpoint fallback. |
| Timeout or process crash | Stop issuing inference; inspect journal, process state, RAM, swap and temperature. Confirm embedding process exit before restarting only that service. |
| Gemma slower while BGE idle | Compare first/warm samples, cooling, swap, available RAM and whether BGE was really idle. Two threads are not CPU isolation; model residency still costs memory. |
| systemd exit 203/EXEC or loader error | Check absolute executable permissions, service user access, `ldd`, and journal. Keep binary and its shared libraries in their working installation. |
| systemd restart limit reached | Fix the cause, then `sudo systemctl reset-failed jarvis-embeddings.service` and start it again. |

## 10. What to send back before integration

First send **section 2's version/help output**. After the runtime checkpoint and manual deployment in sections 3–7, send:

1. Model SHA-256, file size, llama-server version/hash, startup metadata and final bind/port.
2. Health/model responses, vector dimension/norm summaries, and relevant/unrelated scores.
3. Embedding raw timing samples; Gemma baseline, loaded-idle and after-embedding first-content/total timings.
4. Local and LAN results plus CPU/RSS/free-memory/swap/thermal observations, errors or restarts.

Do not paste the full vectors or private prompts. systemd status is only relevant if you chose the optional section 8; it is not a prerequisite for reviewing the manual deployment or beginning later Jarvis integration. No pipeline change or deployment success is implied by this guide.
