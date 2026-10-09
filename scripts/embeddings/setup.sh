#!/bin/sh
set -eu

MODEL_REPOSITORY='Qdrant/bge-small-en-v1.5-onnx-Q'
MODEL_REVISION='aa8f8b060edb00e03bfdd08813a2949946c8ba55'
ORT_VERSION='1.20.1'
ROOT=${JARVIS_EMBEDDING_HOME:-${XDG_DATA_HOME:-"$HOME/.local/share"}/jarvis/embeddings}
DOWNLOADS="$ROOT/downloads"
MODEL_DIR="$ROOT/models/bge-small-en-v1.5-onnx-Q-$MODEL_REVISION"

mkdir -p "$DOWNLOADS" "$ROOT/models" "$ROOT/runtime"

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

fetch_verified() {
    url=$1
    expected=$2
    destination=$3
    temporary="$destination.part.$$"

    if [ -f "$destination" ] && [ "$(sha256 "$destination")" = "$expected" ]; then
        return
    fi

    curl --fail --location --retry 3 --silent --show-error --output "$temporary" "$url"
    actual=$(sha256 "$temporary")
    if [ "$actual" != "$expected" ]; then
        rm -f "$temporary"
        printf 'SHA-256 mismatch for %s: expected %s, got %s\n' "$url" "$expected" "$actual" >&2
        exit 1
    fi
    mv "$temporary" "$destination"
}

fetch_model_file() {
    name=$1
    digest=$2
    url="https://huggingface.co/$MODEL_REPOSITORY/resolve/$MODEL_REVISION/$name?download=true"
    fetch_verified "$url" "$digest" "$MODEL_DIR/$name"
}

mkdir -p "$MODEL_DIR"
fetch_model_file 'model_optimized.onnx' '51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431'
fetch_model_file 'tokenizer.json' 'd241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66'
fetch_model_file 'config.json' '13582bcf2effc85b7bf3d3f5532e686bc1c9ce86bb009d10f0ec33cbe92299dd'
fetch_model_file 'special_tokens_map.json' '5d5b662e421ea9fac075174bb0688ee0d9431699900b90662acd44b2a350503a'
fetch_model_file 'tokenizer_config.json' '0b29c7bfc889e53b36d9dd3e686dd4300f6525110eaa98c76a5dafceb2029f53'

case "$(uname -s):$(uname -m)" in
    Darwin:arm64)
        target='aarch64-apple-darwin'
        archive_dir="onnxruntime-osx-arm64-$ORT_VERSION"
        archive="onnxruntime-osx-arm64-$ORT_VERSION.tgz"
        digest='b678fc3c2354c771fea4fba420edeccfba205140088334df801e7fc40e83a57a'
        library='lib/libonnxruntime.dylib'
        ;;
    Linux:aarch64)
        target='aarch64-unknown-linux-gnu'
        archive_dir="onnxruntime-linux-aarch64-$ORT_VERSION"
        archive="onnxruntime-linux-aarch64-$ORT_VERSION.tgz"
        digest='ae4fedbdc8c18d688c01306b4b50c63de3445cdf2dbd720e01a2fa3810b8106a'
        library='lib/libonnxruntime.so'
        ;;
    *)
        printf 'Unsupported setup target %s:%s; supported targets are ARM64 macOS and ARM64 Linux.\n' "$(uname -s)" "$(uname -m)" >&2
        exit 2
        ;;
esac

archive_path="$DOWNLOADS/$archive"
runtime_dir="$ROOT/runtime/onnxruntime-$ORT_VERSION-$target"
runtime_library="$runtime_dir/$library"
fetch_verified "https://github.com/microsoft/onnxruntime/releases/download/v$ORT_VERSION/$archive" "$digest" "$archive_path"

staging="$ROOT/runtime/.setup-$target-$$"
mkdir -p "$staging"
# ORT tarballs contain one versioned top-level directory (prefixed by ./ in the
# archive). Extract the verified archive and compare the actual library bytes each
# time setup runs, so the runtime identity cannot drift from the pinned release.
tar -xzf "$archive_path" -C "$staging"
archive_library="$staging/$archive_dir/$library"
if [ ! -f "$archive_library" ]; then
    printf 'Pinned ONNX Runtime archive omitted %s\n' "$library" >&2
    rm -rf "$staging"
    exit 1
fi
archive_library_sha=$(sha256 "$archive_library")
mkdir -p "$runtime_dir/$(dirname "$library")"
if [ ! -f "$runtime_library" ] || [ "$(sha256 "$runtime_library")" != "$archive_library_sha" ]; then
    cp "$archive_library" "$runtime_library"
fi
rm -rf "$staging"
identity_temporary="$runtime_dir/runtime.identity.part.$$"
{
    printf 'target=%s\n' "$target"
    printf 'version=%s\n' "$ORT_VERSION"
    printf 'archive_sha256=%s\n' "$digest"
    printf 'library_sha256=%s\n' "$archive_library_sha"
} > "$identity_temporary"
mv "$identity_temporary" "$runtime_dir/runtime.identity"

printf 'Model artifacts are ready at: %s\n' "$MODEL_DIR"
printf 'Set these variables before running Jarvis:\n'
printf '  export JARVIS_EMBEDDING_MODEL_DIR=%s\n' "$MODEL_DIR"
printf '  export ORT_DYLIB_PATH=%s\n' "$runtime_library"
