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

From the repository root:

```sh
cargo test --locked --offline
python3 -B tests/unit/speech/test_supertonic_worker.py
```

The Rust suite uses mocks, including localhost Wiremock fixtures. The Python
suite uses standard-library fixtures; it does not load Supertonic or require
model weights or audio hardware. These checks do not establish Pi playback
acceptance.

Private-module tests remain unit tests even when they use temporary SQLite or
LanceDB files or a localhost HTTP mock. Integration tests exercise the compiled
binary through its public input/output and configuration; they do not include
`src/` modules. No library conversion or visibility widening is required.

Use `cargo test --locked --offline -- --list` after changing registrations to
check discovery. Test implementations and test-only helper bodies must never
remain in `src/`; only conditional declarations and module registrations belong
there. Fixtures and mocks follow the scope of the tests that use them.
