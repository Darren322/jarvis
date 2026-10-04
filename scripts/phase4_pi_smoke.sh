#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--build-only" ) ]]; then
    printf '%s\n' "usage: bash scripts/phase4_pi_smoke.sh [--build-only]" >&2
    exit 2
fi
BUILD_ONLY=0
if [[ $# -eq 1 ]]; then BUILD_ONLY=1; fi
RUN_TIMESTAMP_SGT="$(TZ=Asia/Singapore date '+%Y-%m-%dT%H:%M:%S%z')"

ARTIFACT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/jarvis-phase4-pi-smoke.XXXXXX")"
REVIEWED_SOURCE="$ARTIFACT_DIR/reviewed-source"
BUILD_SOURCE="$ARTIFACT_DIR/build-source"
mkdir -p "$REVIEWED_SOURCE" "$BUILD_SOURCE"
cp "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" "$REVIEWED_SOURCE/"
cp -R "$ROOT/src" "$REVIEWED_SOURCE/src"
cp -R "$REVIEWED_SOURCE/." "$BUILD_SOURCE/"
cp "$ROOT/scripts/phase4_pi_smoke.rs" "$ARTIFACT_DIR/smoke-main.rs"

git -C "$ROOT" rev-parse HEAD > "$ARTIFACT_DIR/git-head.txt"
git -C "$ROOT" status --short --branch --untracked-files=all > "$ARTIFACT_DIR/git-status.txt"
git -C "$ROOT" diff --binary HEAD -- Cargo.toml Cargo.lock src > "$ARTIFACT_DIR/git-source.patch"
git -C "$ROOT" ls-files -- Cargo.toml Cargo.lock src | LC_ALL=C sort | while IFS= read -r relative; do
    file="$REVIEWED_SOURCE/$relative"
    if [[ ! -f "$file" ]]; then
        printf 'tracked source missing from snapshot: %s\n' "$relative" >&2
        exit 1
    fi
    digest="$(shasum -a 256 "$file")"
    printf '%s  %s\n' "${digest%% *}" "$relative"
done > "$ARTIFACT_DIR/source-manifest.sha256"
manifest_digest_line="$(shasum -a 256 "$ARTIFACT_DIR/source-manifest.sha256")"
SOURCE_MANIFEST_SHA256="${manifest_digest_line%% *}"
shasum -a 256 "$ROOT/scripts/phase4_pi_smoke.sh" "$ROOT/scripts/phase4_pi_smoke.rs" > "$ARTIFACT_DIR/helper-manifest.sha256"

cp "$ARTIFACT_DIR/smoke-main.rs" "$BUILD_SOURCE/src/main.rs"
cat >> "$BUILD_SOURCE/src/app.rs" <<'RUST'

impl App {
    pub async fn phase4_smoke_response(
        &self,
        prompt: &str,
    ) -> Result<rig_agent::agent::PromptResponse, Box<dyn std::error::Error>> {
        self.local_llm.health_check().await?;
        Ok(self.assistant.respond(prompt).await?)
    }
}
RUST
awk '
    { print }
    /^\[package\]$/ && !found { print "autobins = false"; found = 1 }
    END { if (!found) exit 1 }
' "$BUILD_SOURCE/Cargo.toml" > "$ARTIFACT_DIR/Cargo.toml.temp"
mv "$ARTIFACT_DIR/Cargo.toml.temp" "$BUILD_SOURCE/Cargo.toml"
cat >> "$BUILD_SOURCE/Cargo.toml" <<'TOML'

[[bin]]
name = "jarvis-phase4-smoke"
path = "src/main.rs"
TOML

PATCH_STATUS=0
diff -u "$REVIEWED_SOURCE/src/app.rs" "$BUILD_SOURCE/src/app.rs" > "$ARTIFACT_DIR/generated-source.patch" || PATCH_STATUS=$?
if (( PATCH_STATUS > 1 )); then
    printf '%s\n' "could not record generated app instrumentation diff" >&2
    exit 1
fi
PATCH_STATUS=0
diff -u "$REVIEWED_SOURCE/src/main.rs" "$BUILD_SOURCE/src/main.rs" >> "$ARTIFACT_DIR/generated-source.patch" || PATCH_STATUS=$?
if (( PATCH_STATUS > 1 )); then
    printf '%s\n' "could not record generated main replacement diff" >&2
    exit 1
fi
PATCH_STATUS=0
diff -u "$REVIEWED_SOURCE/Cargo.toml" "$BUILD_SOURCE/Cargo.toml" >> "$ARTIFACT_DIR/generated-source.patch" || PATCH_STATUS=$?
if (( PATCH_STATUS > 1 )); then
    printf '%s\n' "could not record generated Cargo target diff" >&2
    exit 1
fi
find "$BUILD_SOURCE" -type f -print | LC_ALL=C sort | while IFS= read -r file; do
    digest="$(shasum -a 256 "$file")"
    printf '%s  %s\n' "${digest%% *}" "${file#"$ARTIFACT_DIR"/}"
done > "$ARTIFACT_DIR/generated-source-manifest.sha256"

if [[ "$BUILD_ONLY" -eq 1 ]]; then
    RUN_MODE="build-only verification; no health or model requests"
else
    RUN_MODE="live smoke evidence for the recorded runner host"
fi
{
    printf 'run_mode=%s\n' "$RUN_MODE"
    printf 'git_head_file=git-head.txt; full working-tree status is in git-status.txt\n'
    printf 'reviewed_source=reviewed-source/ (Cargo.toml, Cargo.lock, and full src tree)\n'
    printf 'reviewed_source_digest_sha256=%s\n' "$SOURCE_MANIFEST_SHA256"
    printf 'reviewed_source_digest_scope=Git-tracked Cargo.toml, Cargo.lock, and src files\n'
    printf 'reviewed_source_digest_file=source-manifest.sha256; tracked patch=git-source.patch\n'
    printf 'generated_build_source=build-source/; instrumentation/replacement diff=generated-source.patch\n'
    printf 'generated_build_digest=generated-source-manifest.sha256\n'
    printf 'runner_hostname=%s\n' "$(uname -n)"
    printf 'runner_architecture=%s\n' "$(uname -m)"
    printf 'runner_os=%s\n' "$(uname -s) $(uname -r)"
    printf 'run_timestamp_asia_singapore=%s\n' "$RUN_TIMESTAMP_SGT"
    printf 'inference_host=%s\n' "${JARVIS_SMOKE_INFERENCE_HOST:-unavailable}"
    printf 'server_version=%s\n' "${JARVIS_SMOKE_SERVER_VERSION:-unavailable}"
    printf 'model_build=%s\n' "${JARVIS_SMOKE_MODEL_BUILD:-unavailable}"
    printf 'server_and_model_build_fields=operator-supplied; unavailable unless explicitly set\n'
    printf 'deadlines_seconds=health:3,completion_http:30,assistant_run:30\n'
    printf 'max_tokens=1026; max_turns=2; tool_concurrency=1\n'
    printf 'tool_execution_measurement=canonical transcript evidence only; no tool-body invocation counter\n'
    printf 'effective_model=written by AppConfig::load to configured-model.txt during live run\n'
    printf 'configured_model_file=configured-model.txt; endpoint URLs are intentionally omitted\n'
} > "$ARTIFACT_DIR/provenance.txt"

printf 'Artifacts: %s\n' "$ARTIFACT_DIR"
printf 'source_manifest_sha256=%s\n' "$SOURCE_MANIFEST_SHA256"
if [[ -n "${JARVIS_SMOKE_EXPECTED_SOURCE_SHA256:-}" && "$SOURCE_MANIFEST_SHA256" != "$JARVIS_SMOKE_EXPECTED_SOURCE_SHA256" ]]; then
    printf 'source manifest SHA256 mismatch: expected=%s actual=%s; build and live cases were skipped\n' \
        "$JARVIS_SMOKE_EXPECTED_SOURCE_SHA256" "$SOURCE_MANIFEST_SHA256" >&2
    exit 1
fi
if ! (cd "$BUILD_SOURCE" && cargo build --locked --offline --manifest-path "$BUILD_SOURCE/Cargo.toml" --target-dir "$ROOT/target" --bin jarvis-phase4-smoke) > "$ARTIFACT_DIR/cargo-build.log" 2>&1; then
    printf '%s\n' "offline smoke-helper build failed; see cargo-build.log" >&2
    exit 1
fi
printf '%s\n' "build=ok"
if [[ "$BUILD_ONLY" -eq 1 ]]; then
    printf '%s\n' "mode=build-only; no Pi or real-model pass is claimed"
    exit 0
fi

BIN="$ROOT/target/debug/jarvis-phase4-smoke"
if [[ ! -x "$BIN" ]]; then
    printf '%s\n' "built binary not found at target/debug/jarvis-phase4-smoke; see cargo-build.log" >&2
    exit 1
fi
printf 'live_cases_started_at_asia_singapore=%s\n' "$(TZ=Asia/Singapore date '+%Y-%m-%dT%H:%M:%S%z')" >> "$ARTIFACT_DIR/provenance.txt"

run_case() {
    local case_name="$1"
    if (cd "$ROOT" && JARVIS_SMOKE_MODEL_FILE="$ARTIFACT_DIR/configured-model.txt" "$BIN" "$case_name") \
        > "$ARTIFACT_DIR/$case_name.stdout" 2> "$ARTIFACT_DIR/$case_name.stderr"; then
        printf 'case=%s result=accepted\n' "$case_name"
        cat "$ARTIFACT_DIR/$case_name.stdout"
        return 0
    fi
    printf 'case=%s result=failed; details retained in %s/%s.stderr\n' "$case_name" "$ARTIFACT_DIR" "$case_name" >&2
    return 1
}

STATUS_RESULT=0
UNSUPPORTED_RESULT=0
run_case status || STATUS_RESULT=1
run_case unsupported || UNSUPPORTED_RESULT=1
if [[ ! -f "$ARTIFACT_DIR/configured-model.txt" ]]; then
    printf '%s\n' "effective model was not recorded; configuration may have failed" >&2
    exit 1
fi
if [[ "$STATUS_RESULT" -ne 0 || "$UNSUPPORTED_RESULT" -ne 0 ]]; then
    printf '%s\n' "one or more smoke cases failed; inspect the retained artifacts" >&2
    exit 1
fi
printf 'configured_model=%s\n' "$(cat "$ARTIFACT_DIR/configured-model.txt")"
printf '%s\n' "live smoke evidence is for the recorded runner host; inspect provenance.txt before treating it as Pi evidence"
