# Jarvis tests

All test code lives here, separate from production code.

- `unit/`: Rust and Python unit tests, helpers, fixtures and mocks.
- `integration/`: black-box workflows that run the compiled binary. Nested
  entry points are registered explicitly in `Cargo.toml`.

## Where a unit test lives

`tests/unit/` mirrors `src/`. Tests for a source file sit in the matching
directory and are named after the module:

| Source | Tests |
| --- | --- |
| `src/storage/schema.rs` | `tests/unit/storage/schema_tests.rs` |
| `src/features/memory/mod.rs` | `tests/unit/features/memory/memory_tests.rs` |
| `src/interfaces/cli/repl/mod.rs` | `tests/unit/interfaces/cli/repl/repl_tests.rs` |

The source file registers its tests with one declaration, always named `tests`:

```rust
#[cfg(test)]
#[path = "../../tests/unit/storage/schema_tests.rs"]
mod tests;
```

The test module is still a child of the code it tests, so it can use private
items without widening production visibility. The
[Rust reference](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute)
explains how `#[path]` picks a file without changing the logical module.

## What to test

Test behavior at its boundaries rather than repeating near-identical cases:
for every limit, check the last accepted value and the first rejected one.
Test a rule once, in the layer that owns it. For example, the pure policy
function covers every policy rule; the Rig hook tests cover only what the hook
adds, such as a rejected tool never executing.

## Running

```sh
cargo test --locked --offline
python3 -B tests/unit/features/speech/supertonic_worker/test_supertonic_worker.py
```

Model tests always use the streaming path production uses: Rig mocks are
scripted with `MockCompletionModel::from_stream_turns`, and Wiremock fixtures
return OpenAI-style server-sent events. Don't add tests through Rig's
non-streaming `.run()` or `.completion()`; they would test code Jarvis never
calls.

The Rust suite uses mocks, temporary SQLite files (with the statically linked
sqlite-vec extension), and localhost Wiremock servers; it never calls a real
model. The Python suite uses standard-library fixtures and needs no Supertonic
weights or audio hardware. Neither establishes Pi playback acceptance.

After changing registrations, compare `cargo test --locked --offline -- --list`
before and after to confirm every test is still discovered.
