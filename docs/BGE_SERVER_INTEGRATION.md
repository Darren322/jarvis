# Integrating the remote BGE embedding server

This guide is for **after** you have run [the deployment benchmark](TWO_PI_BGE_DEPLOYMENT.md) on the 16 GB Pi and the BGE `llama-server` answers on `/v1/embeddings`. It moves embedding off the 8 GB Pi and removes ONNX Runtime from Jarvis.

Before you start, you should have these results from the benchmark:

- [ ] `llama-server --version`, and the SHA-256 of `bge-small-en-v1.5-q8_0.gguf`
- [ ] Vectors with 384 dimensions and an L2 norm of about 1.0
- [ ] Warm query latency over the LAN, measured from the 8 GB Pi
- [ ] The URL the 8 GB Pi can reach, for example `http://jarvis-ai.local:8081`

## What changes, and what doesn't

Only the inside of `Embedder` (`src/features/memory/embedder.rs`) changes. Everything around it stays the same:

| Stays the same | Why |
| --- | --- |
| `MemoryService`, recall, idle pass, CLI | They only call `Embedder::embed`, `state`, `fingerprint` and `wait_until_idle` |
| SQLite schema v4 | `memory_vectors` is still `float[384]` |
| Keyword-only fallback | A server error becomes `EmbedError::Unavailable`, and recall degrades to keywords as it does today |
| One request at a time | The semaphore stays; it matches `--parallel 1` on the server |

**Re-embedding happens automatically.** GGUF Q8_0 and the current ONNX INT8 model produce different vectors. Each memory row stores the fingerprint of the model that embedded it, so a new fingerprint marks every row as pending, and idle passes re-embed them. Until then, recall matches only against vectors from the new model, plus keywords. You don't need to run `/memory rebuild`.

## Step 1: configuration

In `src/config/mod.rs`, add an embedding section next to `LocalLlmConfig`:

```rust
pub struct EmbeddingConfig {
    pub base_url: String, // e.g. http://jarvis-ai.local:8081/v1
    pub model: String,    // the --alias you started llama-server with
}
```

Load it with `required_env("JARVIS_EMBEDDING_BASE_URL")` and `required_env("JARVIS_EMBEDDING_MODEL")`. Then remove `embedding_model_dir` and `DEFAULT_EMBEDDING_MODEL_DIR`. Update `.env.example` the same way: delete `JARVIS_EMBEDDING_MODEL_DIR` and `ORT_DYLIB_PATH`, and add the two new variables.

## Step 2: reuse the local-endpoint protections

`src/clients/local_llm.rs` already rejects non-local hosts, credentials in URLs, proxies and redirects, and it checks every DNS answer through `LocalEndpointResolver`. The embedding client needs exactly the same protections. Move these items into `src/clients/local_endpoint.rs` as `pub(crate)`, so both clients can share them:

- `validate_local_endpoint`
- `LocalEndpointError`
- `LocalEndpointResolver`
- `validate_resolved_addresses`

This is a move, not a rewrite: the existing tests should pass unchanged.

## Step 3: the HTTP client

Create `src/clients/remote_embeddings.rs`. Write a direct `reqwest` call rather than going through Rig's embedding model, because you need a cap on the response size and the same endpoint protections as above.

```rust
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
const DIMENSIONS: usize = 384;
const MAX_RESPONSE_BYTES: usize = 64 * 1024; // one 384-float vector as JSON is ~8 KiB

/// Identifies the vector recipe. Changing any part re-embeds all memories.
pub(crate) const MODEL_FINGERPRINT: &str = "bge-small-en-v1.5-gguf:q8_0:\
    d32f8c040ea3b516330eeb75b72bcc2d3a780ab7:\
    sha256=ec38e8da142596baa913124ae50550de284b6916bf59577ef2f0cb9660c2f514:\
    query-prefix-v1:raw-passage:cls:l2";

pub(crate) struct RemoteEmbeddings {
    http: reqwest::Client,
    url: reqwest::Url, // base_url joined with "embeddings"
    model: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RemoteEmbeddingsError {
    #[error("embedding request failed")]
    Http(#[from] reqwest::Error),
    #[error("embedding response exceeded {MAX_RESPONSE_BYTES} bytes")]
    TooLarge,
    #[error("embedding response was not valid JSON")]
    Json(#[from] serde_json::Error),
    #[error("embedding had {0} dimensions, expected {DIMENSIONS}")]
    WrongDimensions(usize),
    #[error("embedding was not a finite unit vector")]
    NotNormalized,
}
```

Build the client the way `LocalLlm::new` does: `.no_proxy()`, `.dns_resolver(LocalEndpointResolver)`, `.redirect(Policy::none())`, `.retry(reqwest::retry::never())`, and a timeout. Start with 8 seconds, then adjust it to your measured LAN latency.

The request and response:

```rust
#[derive(serde::Serialize)]
struct Request<'a> { model: &'a str, input: &'a str, encoding_format: &'static str }

#[derive(serde::Deserialize)]
struct EmbeddingResponse { data: Vec<Item> }
#[derive(serde::Deserialize)]
struct Item { embedding: Vec<f32> }
```

Read the body in chunks so a misbehaving server can't make Jarvis buffer an unbounded response:

```rust
let mut response = self.http.post(self.url.clone()).json(&request).send().await?
    .error_for_status()?;
let mut body = Vec::new();
while let Some(chunk) = response.chunk().await? {
    if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
        return Err(RemoteEmbeddingsError::TooLarge);
    }
    body.extend_from_slice(&chunk);
}
let mut parsed: EmbeddingResponse = serde_json::from_slice(&body)?;
```

Then validate before returning. Check that there's exactly one item, that it has 384 values, that every value is finite, and that the L2 norm is within 0.001 of 1.0. Expose two methods:

- `embed_query(&self, text)`: sends `QUERY_PREFIX + text`.
- `embed_document(&self, text)`: sends the raw text.

## Step 4: swap the inside of `Embedder`

In `src/features/memory/embedder.rs`:

1. **State.** Replace `model: Arc<OnceLock<Option<LocalEmbeddings>>>` with `client: RemoteEmbeddings` plus a readiness flag (for example an `AtomicU8` holding `EmbedderState`).
2. **`Embedder::load`.** It becomes `Embedder::connect(config)`. Spawn one `GET /health` (on the server root, not `/v1`) to move from `Loading` to `Ready` or `Unavailable`. It must not block startup.
3. **`embed`.** It keeps its signature. Acquire the semaphore permit as now, then `await` the HTTP call directly instead of using `spawn_blocking`.
   - Map a transport error to `EmbedError::Unavailable` and mark the state `Unavailable`. Mark it `Ready` again after the next success, so the server coming back heals recall without a restart.
   - Map a client timeout to `EmbedError::TimedOut`.
4. **`fingerprint`.** It returns `remote_embeddings::MODEL_FINGERPRINT`.
5. **Errors.** Replace `EmbedError::Failed(LocalEmbeddingsError)` with `Failed(RemoteEmbeddingsError)`. Delete `EmbedError::Task`, since there's no blocking task any more.

`MemoryService::open(repository, model_dir)` becomes `open(repository, embedding_config)`. `App` passes `&config.embedding`.

A timeout means Jarvis stopped waiting, not that the server stopped computing. The server only handles one request at a time, so a request that keeps running after a timeout makes the next one wait behind it. That's acceptable at Jarvis's request rate, but it's why the benchmark measures warm latency.

## Step 5: delete the local model

- Delete `src/clients/local_embeddings.rs` and `scripts/embeddings/`.
- In `Cargo.toml`, remove `fastembed` and `libloading`. Keep `sha2`: imports use it.
- Remove the `fastembed`/ONNX notes from `README.md` and `tests/README.md`.
- On the 8 GB Pi, delete `data/embeddings/` and the ONNX Runtime library once the new build is running.

## Step 6: tests

Add `tests/unit/clients/remote_embeddings_tests.rs`, using `wiremock` the way `openai_adapter_tests.rs` does:

- [ ] A 384-value unit vector is accepted. The query request body contains the prefix exactly once; the document request body contains no prefix.
- [ ] Wrong dimensions, a NaN value, a non-unit norm, and zero or two `data` items are each rejected.
- [ ] A body larger than `MAX_RESPONSE_BYTES` is rejected without being parsed.
- [ ] A 503 response maps to an error. In an `Embedder` test, `recall` then returns a keyword-only warning, matching the existing `recall_falls_back_to_keywords_when_the_embedder_is_unavailable` test.
- [ ] A non-local URL fails validation before any request is sent.

The existing `tests/unit/memory/service_tests.rs` should only need a config value in place of a model directory.

## Step 7: verify on the Pis

1. Back up `data/jarvis.sqlite3`.
2. Deploy, then run `/memory status`. The embedder should say `ready`, and "awaiting embedding" should equal the total number of memories, because the fingerprint changed.
3. Leave Jarvis idle and watch the pending count reach 0. Check Gemma's chat latency while this runs; idle embedding and chat share the 16 GB Pi.
4. Ask something you discussed earlier, worded differently. It should be recalled with no keyword-only warning.
5. Stop the BGE server and ask again. You should get a keyword-only warning, and the chat itself should still work. Start the server again: the next request recovers without restarting Jarvis.

**Rollback:** revert the commit. The fingerprint changes back, and idle passes re-embed with the local model.
