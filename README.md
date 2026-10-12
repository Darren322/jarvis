# Jarvis

Jarvis is a local-first Rust assistant. `src/main.rs` handles runtime setup and configuration loading. `src/app.rs` assembles providers and feature services. The terminal boundary lives in `src/interfaces/cli/`.

## Code map

- `src/features/assistant/` owns one bounded Rig run and its policy and report.
- `src/features/conversation/` owns selected session history and archive coordination.
- `src/features/memory/` owns memory operations, extraction lifecycle, recall, and the derived LanceDB index.
- `src/features/speech/` owns synthesis, playback, and worker cleanup.
- `src/clients/` owns provider construction.
- `src/storage/` owns SQLite schema and transactions; `src/storage/memory_repository/` contains the memory repository and SQL operations.
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

## Local memory files

SQLite at `data/jarvis.sqlite3` is the canonical archive, source, evidence, correction, suppression, and projection-queue store. `JARVIS_ARCHIVE_PATH` can select another local path. LanceDB under `data/memory-index` is a rebuildable search projection; the local embedding model defaults to `data/embeddings`. The path overrides are `JARVIS_MEMORY_INDEX_DIR` and `JARVIS_EMBEDDING_MODEL_DIR`.

Exit or close the application normally before maintenance. For a consistent SQLite backup, use the SQLite CLI backup command instead of copying a live file:

```sh
sqlite3 data/jarvis.sqlite3 '.backup data/jarvis-backup.sqlite3'
```

Jarvis only quarantines an index directory with its exact owner marker. Keep custom or unmarked directories intact; `/memory rebuild` queues the derived projection from SQLite. See [Memory storage operations](docs/MEMORY_STORAGE_OPERATIONS.md) for shutdown, backup, and recovery boundaries. The isolated synthetic LanceDB 0.30 authority/outbox probe runs with `cargo run --locked --offline --example lancedb_authority_probe`; it is not a production migration or a full recovery test. It observed a historical table restore resurrecting a previously suppressed row until tombstone replay repaired it. SQLite remains authoritative, and the user chose to skip a production migration.
