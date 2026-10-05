# Jarvis tests

All test implementation files live here, separate from production code.

- `unit/`: Rust tests for App, storage, conversation, Assistant, clients, tools,
  and TTS.
- `python/`: Python worker protocol and asset-verification tests.

Rust unit tests remain child modules of the code they exercise. Production
modules contain only a small `#[cfg(test)]` / `#[path]` declaration pointing here.
This preserves private access, existing test names, and the single test harness.
The [Rust module reference](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute)
describes how `#[path]` selects a file without changing its logical module.

From the repository root:

```sh
cargo test --locked --offline
python3 -B tests/python/test_supertonic_worker.py
```

The Rust suite uses mocks, including localhost Wiremock fixtures. The Python
suite uses standard-library fixtures; it does not load Supertonic or require
model weights or audio hardware. These checks do not establish Pi playback
acceptance.

Future tests of the public application interface can be separate Cargo
integration tests. The existing private-module tests remain unit tests; no
library conversion or public visibility change is required for this layout.
