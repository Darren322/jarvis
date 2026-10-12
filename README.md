# Jarvis

Jarvis is a local-first Rust assistant. `src/main.rs` handles runtime setup and configuration loading. `src/app.rs` assembles providers and feature services. The terminal boundary lives in `src/interfaces/cli/`.

## Code map

- `src/features/assistant/` owns one bounded Rig run and its policy and report.
- `src/features/conversation/` owns selected session history and archive coordination.
- `src/features/memory/` owns recall (hybrid keyword + vector search), imports, forgetting, and idle embedding.
- `src/features/speech/` owns synthesis, playback, and worker cleanup.
- `src/clients/` owns provider construction.
- `src/storage/` owns the SQLite schema, migrations, chunking, and transactions; `src/storage/memory_repository/` contains the memory SQL.
- `src/config/` owns settings; `src/tools/` owns capability adapters.
- Rust and Python unit tests live under `tests/unit/`; black-box workflows live under `tests/integration/`.

## Build and verify

```sh
cargo fmt --check
cargo check --locked --offline --all-targets
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
git diff --check
```

See [tests/README.md](tests/README.md) for test boundaries and the separate Python worker check.

## Local memory

Everything durable is in one SQLite file, `data/jarvis.sqlite3` (override with `JARVIS_ARCHIVE_PATH`). It holds the archive, chunked memories, an FTS5 keyword index, and a sqlite-vec vector index; recall fuses both. The local embedding model defaults to `data/embeddings` (`JARVIS_EMBEDDING_MODEL_DIR`). See [Memory storage operations](docs/MEMORY_STORAGE_OPERATIONS.md) for commands, the v4 upgrade, and backups.

## Plans

[docs/ROADMAP.md](docs/ROADMAP.md) has the current priorities. [docs/BGE_SERVER_INTEGRATION.md](docs/BGE_SERVER_INTEGRATION.md) explains moving embeddings to the 16 GB Pi.
