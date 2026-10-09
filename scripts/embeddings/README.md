# Local embedding assets

`manifest.json` pins the FastEmbed model files, preprocessing, and the CPU ONNX Runtime release. The model and native library are setup-time artifacts; the application reads explicit paths and has no model-host feature enabled.

Run `./scripts/embeddings/setup.sh` once on the target machine. It supports Apple ARM64 macOS and Linux ARM64, downloads only the pinned artifacts, checks each SHA-256, and prints the two environment variables to set. Keep the resulting model/runtime folders outside Git.

The code uses FastEmbed 4.9.1 with `default-features = false` and only `ort-load-dynamic`. The model path is loaded through `TextEmbedding::try_new_from_user_defined`, never FastEmbed's hub-backed constructor. ONNX Runtime 1.20.1 is loaded from `ORT_DYLIB_PATH`; the matching Rust binding is `ort`/`ort-sys` 2.0.0-rc.9. Setup extracts the requested library from the SHA-256-verified release archive, hashes the extracted library, and writes a local `runtime.identity` receipt. The Rust loader rechecks that library hash and calls `OrtGetApiBase` to require the exact `1.20.1` runtime version before constructing FastEmbed. The runtime archive hash is part of the vector fingerprint, so changing the runtime rebuilds the projection. Runtime 1.20.1 includes the CPU `SkipLayerNorm` fixes needed by this quantized BGE model. Runtime 1.20.0 failed at inference with a missing LayerNorm weight, as reported in [FastEmbed issue #385](https://github.com/qdrant/fastembed/issues/385); the [1.20.1 release notes](https://github.com/microsoft/onnxruntime/releases/tag/v1.20.1) list the corresponding CPU fix.

The model is English-first: 384 dimensions, a 512-token maximum, CLS pooling, static quantization, and FastEmbed L2 normalization. BGE's retrieval query prefix is applied only to queries; imported memory passages use no prefix. Both use the same model and tokenizer. Changing the revision, tokenizer files, prefix, pooling, quantization, normalization, or dimension changes the vector identity and requires an index rebuild.

## Local smoke

After setting the variables printed by the setup script, run from the repository root:

```sh
CARGO_TARGET_DIR=target/phase7e-checkpoint \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
cargo run --locked --offline --example memory_backend_probe
```

The first part creates a temporary LanceDB table with synthetic rows and fake vectors, builds an FTS index, and executes the native full-text plus vector query with LanceDB's RRF reranker. When `JARVIS_EMBEDDING_MODEL_DIR` is set, it also loads the local ONNX/tokenizer artifacts and checks a small English paraphrase against two unrelated passages. No real user data, completion model, or runtime download is involved.

## Raspberry Pi 5 / Debian 13 smoke

Run these steps on the Pi itself; an ARM64 artifact existing and a macOS build succeeding do not establish Pi compatibility or latency.

1. Confirm `uname -m` prints `aarch64`; run `./scripts/embeddings/setup.sh` and export its printed variables.
2. Disconnect networking after setup, then build/run the smoke with the command above. The native query should return the synthetic Kyoto rows; the local model path should report 384 dimensions and rank its Kyoto paraphrase above the soup and Paris passages.
3. Repeat after restarting the process to check that artifacts are loaded from the same explicit local paths. Record the Pi model-load time, first inference time, steady inference time, peak memory, and any runtime loader errors as observed measurements; this checkpoint does not set performance acceptance thresholds.

The smoke does not call the configured chat model or extract personal memories. It verifies library/runtime construction only; full memory retrieval acceptance remains separate.
