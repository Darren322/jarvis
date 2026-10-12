# Jarvis tests

All test implementation files live here, separate from production code.

- `unit/`: Rust and Python unit tests, helpers, fixtures and mocks organized by feature.
- `integration/`: black-box binary workflows and their own fixtures/mocks.
  Nested entry points are registered explicitly in `Cargo.toml`.

Rust unit tests remain child modules of the code they exercise. Production
modules contain only a small `#[cfg(test)]` / `#[path]` declaration pointing here.
This preserves private access without exposing production internals. The binary
unit-test target and separate integration targets run through Cargo.
The [Rust module reference](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute)
describes how `#[path]` selects a file without changing its logical module.

Assistant, conversation, memory, and speech roots live under
`src/features/{assistant,conversation,memory,speech}`. Their test source files
stay in `tests/unit/` during module moves; update only the private-module path
hooks and crate imports required by the new logical paths. Keep integration
tests under `tests/integration/` and preserve Cargo target names.

Conversation history tests are registered from
`src/features/conversation/history.rs`; session and archive-flow tests are
registered from `src/features/conversation/session.rs`. Their implementations
remain in `tests/unit/conversation/history_tests.rs` and
`tests/unit/conversation/conversation_tests.rs`, respectively.

The memory repository implementation lives in `src/storage/memory_repository/`.
Its unit-test files remain under `tests/unit/storage/memory/`; the existing
private hooks preserve their access without widening production visibility.

From the repository root:

```sh
cargo test --locked --offline
python3 -B tests/unit/speech/test_supertonic_worker.py
```

The Rust suite uses mocks, including localhost Wiremock fixtures. The Python
suite uses standard-library fixtures; it does not load Supertonic or require
model weights or audio hardware. These checks do not establish Pi playback
acceptance.

Private-module tests remain unit tests even when they use temporary SQLite
files or a localhost HTTP mock. Memory tests run against real SQLite with the
statically linked sqlite-vec extension; the v3-to-v4 migration is covered by
`unit/storage/schema_tests.rs`. Integration tests exercise the compiled
binary through its public input/output and configuration; they do not include
`src/` modules. No library conversion or visibility widening is required.

Use `cargo test --locked --offline -- --list` after changing registrations to
check discovery. Test implementations and test-only helper bodies must never
remain in `src/`; only conditional declarations and module registrations belong
there. Fixtures and mocks follow the scope of the tests that use them.
