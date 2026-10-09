//! Loads the pinned text embedder from an explicitly provisioned local model directory.
//!
//! Model and ONNX Runtime acquisition belong to `scripts/embeddings/`; constructing this
//! client reads only those files and never contacts a model host.

use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
};

use fastembed::{
    EmbeddingModel, InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding,
    TokenizerFiles, UserDefinedEmbeddingModel,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

const DIMENSIONS: usize = 384;
const MAX_TOKENS: usize = 512;
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub const MODEL_FINGERPRINT: &str = "bge-small-en-v1.5-q:aa8f8b060edb00e03bfdd08813a2949946c8ba55:sha256=51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431:query-prefix-v1:raw-passage:cls:int8:l2:fastembed-4.9.1:ort-1.20.1:runtime-archive=b678fc3c2354c771fea4fba420edeccfba205140088334df801e7fc40e83a57a";
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
pub const MODEL_FINGERPRINT: &str = "bge-small-en-v1.5-q:aa8f8b060edb00e03bfdd08813a2949946c8ba55:sha256=51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431:query-prefix-v1:raw-passage:cls:int8:l2:fastembed-4.9.1:ort-1.20.1:runtime-archive=ae4fedbdc8c18d688c01306b4b50c63de3445cdf2dbd720e01a2fa3810b8106a";
#[cfg(not(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "aarch64", target_os = "linux")
)))]
pub const MODEL_FINGERPRINT: &str = "unsupported-runtime-target";

const ARTIFACTS: [(&str, usize, &str); 5] = [
    (
        "model_optimized.onnx",
        66_465_124,
        "51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431",
    ),
    (
        "tokenizer.json",
        711_396,
        "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    ),
    (
        "config.json",
        706,
        "13582bcf2effc85b7bf3d3f5532e686bc1c9ce86bb009d10f0ec33cbe92299dd",
    ),
    (
        "special_tokens_map.json",
        695,
        "5d5b662e421ea9fac075174bb0688ee0d9431699900b90662acd44b2a350503a",
    ),
    (
        "tokenizer_config.json",
        1_242,
        "0b29c7bfc889e53b36d9dd3e686dd4300f6525110eaa98c76a5dafceb2029f53",
    ),
];

pub struct LocalEmbeddings {
    model: TextEmbedding,
}

#[derive(Debug, Error)]
pub enum LocalEmbeddingsError {
    #[error("ORT_DYLIB_PATH must point to a locally installed ONNX Runtime shared library")]
    RuntimeNotConfigured,
    #[error("ONNX Runtime shared library does not exist: {0}")]
    RuntimeNotFound(PathBuf),
    #[error("ONNX Runtime library does not match the locally provisioned pinned identity")]
    RuntimeIdentityInvalid,
    #[error("ONNX Runtime version mismatch: expected {expected}, got {actual}")]
    RuntimeVersion {
        expected: &'static str,
        actual: String,
    },
    #[error("failed to read local embedding artifact at {path}")]
    ArtifactIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "local embedding artifact {path} failed its pinned integrity check (bytes={actual_bytes}, sha256={actual_sha256})"
    )]
    ArtifactIntegrity {
        path: PathBuf,
        actual_bytes: usize,
        actual_sha256: String,
    },
    #[error("failed to initialize or run the pinned local embedding model")]
    Initialization(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("FastEmbed returned {actual} dimensions, expected {expected}")]
    WrongDimensions { actual: usize, expected: usize },
    #[error("FastEmbed returned no vector for one input text")]
    MissingVector,
}

impl LocalEmbeddings {
    /// Load the manifest-pinned ONNX and tokenizer files from disk.
    pub fn load_from_dir(model_dir: &Path) -> Result<Self, LocalEmbeddingsError> {
        // LEARNING: `ort-load-dynamic` resolves this path at runtime. Requiring it before
        // creating a Session makes the separately provisioned native library explicit and
        // prevents FastEmbed's download-enabled convenience constructor from being used here.
        let runtime_path = env::var_os("ORT_DYLIB_PATH")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(LocalEmbeddingsError::RuntimeNotConfigured)?;
        if !runtime_path.is_file() {
            return Err(LocalEmbeddingsError::RuntimeNotFound(runtime_path));
        }
        verify_runtime(&runtime_path)?;

        let onnx_file = read_artifact(model_dir, ARTIFACTS[0])?;
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: read_artifact(model_dir, ARTIFACTS[1])?,
            config_file: read_artifact(model_dir, ARTIFACTS[2])?,
            special_tokens_map_file: read_artifact(model_dir, ARTIFACTS[3])?,
            tokenizer_config_file: read_artifact(model_dir, ARTIFACTS[4])?,
        };
        let model = UserDefinedEmbeddingModel::new(onnx_file, tokenizer_files)
            .with_pooling(Pooling::Cls)
            .with_quantization(QuantizationMode::Static);

        // This enum identifies the selected manifest model for diagnostics. The local user-
        // supplied bytes above are what FastEmbed loads; this call does not look up or download it.
        let model_id = EmbeddingModel::BGESmallENV15Q;
        let model_info = TextEmbedding::get_model_info(&model_id)
            .map_err(|error| LocalEmbeddingsError::Initialization(error.into_boxed_dyn_error()))?;
        if model_info.dim != DIMENSIONS {
            return Err(LocalEmbeddingsError::WrongDimensions {
                actual: model_info.dim,
                expected: DIMENSIONS,
            });
        }

        let model = TextEmbedding::try_new_from_user_defined(
            model,
            InitOptionsUserDefined::new().with_max_length(MAX_TOKENS),
        )
        .map_err(|error| LocalEmbeddingsError::Initialization(error.into_boxed_dyn_error()))?;

        Ok(Self { model })
    }

    /// Embed a search query using the BGE retrieval instruction before tokenization.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, LocalEmbeddingsError> {
        self.embed_encoded_text(&format!("{QUERY_PREFIX}{query}"))
    }

    /// Embed a memory passage without a query instruction.
    pub fn embed_document(&self, text: &str) -> Result<Vec<f32>, LocalEmbeddingsError> {
        self.embed_encoded_text(text)
    }

    fn embed_encoded_text(&self, text: &str) -> Result<Vec<f32>, LocalEmbeddingsError> {
        // LEARNING: Query and passage instructions differ by the model's retrieval recipe, while
        // both still pass through this one pinned tokenizer, ONNX model, CLS pool, and L2 norm.
        let mut vectors = self
            .model
            .embed(vec![text], Some(1))
            .map_err(|error| LocalEmbeddingsError::Initialization(error.into_boxed_dyn_error()))?;
        let vector = vectors.pop().ok_or(LocalEmbeddingsError::MissingVector)?;
        if vector.len() != DIMENSIONS {
            return Err(LocalEmbeddingsError::WrongDimensions {
                actual: vector.len(),
                expected: DIMENSIONS,
            });
        }
        Ok(vector)
    }
}

fn verify_runtime(runtime_path: &Path) -> Result<(), LocalEmbeddingsError> {
    let (target, archive_sha256) = runtime_pin()?;
    let runtime_dir = runtime_path
        .parent()
        .and_then(Path::parent)
        .ok_or(LocalEmbeddingsError::RuntimeIdentityInvalid)?;
    let identity_path = runtime_dir.join("runtime.identity");
    let identity = fs::read_to_string(&identity_path)
        .map_err(|_| LocalEmbeddingsError::RuntimeIdentityInvalid)?;
    let field = |name: &str| {
        identity.lines().find_map(|line| {
            line.split_once('=')
                .filter(|(key, _)| *key == name)
                .map(|(_, value)| value)
        })
    };
    let library_sha256 =
        field("library_sha256").ok_or(LocalEmbeddingsError::RuntimeIdentityInvalid)?;
    if field("target") != Some(target)
        || field("version") != Some("1.20.1")
        || field("archive_sha256") != Some(archive_sha256)
        || hash_file(runtime_path)? != library_sha256
    {
        return Err(LocalEmbeddingsError::RuntimeIdentityInvalid);
    }

    let library = unsafe { libloading::Library::new(runtime_path) }
        .map_err(|_| LocalEmbeddingsError::RuntimeIdentityInvalid)?;
    #[repr(C)]
    struct OrtApiBase {
        get_api: Option<unsafe extern "C" fn(u32) -> *const std::ffi::c_void>,
        get_version_string: Option<unsafe extern "C" fn() -> *const std::ffi::c_char>,
    }
    type GetApiBase = unsafe extern "C" fn() -> *const OrtApiBase;

    // LEARNING: Runtime archive and library hashes establish the provisioned binary, while
    // ONNX Runtime's own C API confirms the exact loaded 1.20.1 version before FastEmbed builds
    // a session. The marker is emitted only after setup extracts the pinned archive.
    let version = unsafe {
        let get_api_base = library
            .get::<GetApiBase>(b"OrtGetApiBase")
            .map_err(|_| LocalEmbeddingsError::RuntimeIdentityInvalid)?;
        let base = get_api_base();
        if base.is_null() {
            return Err(LocalEmbeddingsError::RuntimeIdentityInvalid);
        }
        let get_version = (*base)
            .get_version_string
            .ok_or(LocalEmbeddingsError::RuntimeIdentityInvalid)?;
        let version_ptr = get_version();
        if version_ptr.is_null() {
            return Err(LocalEmbeddingsError::RuntimeIdentityInvalid);
        }
        std::ffi::CStr::from_ptr(version_ptr)
            .to_string_lossy()
            .into_owned()
    };
    if version != "1.20.1" {
        return Err(LocalEmbeddingsError::RuntimeVersion {
            expected: "1.20.1",
            actual: version,
        });
    }
    Ok(())
}

fn runtime_pin() -> Result<(&'static str, &'static str), LocalEmbeddingsError> {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    {
        Ok((
            "aarch64-apple-darwin",
            "b678fc3c2354c771fea4fba420edeccfba205140088334df801e7fc40e83a57a",
        ))
    }
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    {
        Ok((
            "aarch64-unknown-linux-gnu",
            "ae4fedbdc8c18d688c01306b4b50c63de3445cdf2dbd720e01a2fa3810b8106a",
        ))
    }
    #[cfg(not(any(
        all(target_arch = "aarch64", target_os = "macos"),
        all(target_arch = "aarch64", target_os = "linux")
    )))]
    {
        Err(LocalEmbeddingsError::RuntimeIdentityInvalid)
    }
}

fn hash_file(path: &Path) -> Result<String, LocalEmbeddingsError> {
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(|source| LocalEmbeddingsError::ArtifactIo {
        path: path.to_owned(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| LocalEmbeddingsError::ArtifactIo {
                path: path.to_owned(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn read_artifact(
    model_dir: &Path,
    (name, expected_bytes, expected_sha256): (&str, usize, &str),
) -> Result<Vec<u8>, LocalEmbeddingsError> {
    let path = model_dir.join(name);
    let metadata =
        fs::symlink_metadata(&path).map_err(|source| LocalEmbeddingsError::ArtifactIo {
            path: path.clone(),
            source,
        })?;
    if !metadata.file_type().is_file() || metadata.len() != expected_bytes as u64 {
        return Err(LocalEmbeddingsError::ArtifactIntegrity {
            path,
            actual_bytes: usize::try_from(metadata.len()).unwrap_or(usize::MAX),
            actual_sha256: "not computed for a non-regular or wrong-sized file".to_owned(),
        });
    }
    let file = fs::File::open(&path).map_err(|source| LocalEmbeddingsError::ArtifactIo {
        path: path.clone(),
        source,
    })?;
    if !file
        .metadata()
        .map_err(|source| LocalEmbeddingsError::ArtifactIo {
            path: path.clone(),
            source,
        })?
        .is_file()
    {
        return Err(LocalEmbeddingsError::ArtifactIntegrity {
            path,
            actual_bytes: 0,
            actual_sha256: "not computed for a non-regular file".to_owned(),
        });
    }
    let mut bytes = Vec::with_capacity(expected_bytes);
    file.take(expected_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| LocalEmbeddingsError::ArtifactIo {
            path: path.clone(),
            source,
        })?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&bytes));
    if bytes.len() != expected_bytes || actual_sha256 != expected_sha256 {
        return Err(LocalEmbeddingsError::ArtifactIntegrity {
            path,
            actual_bytes: bytes.len(),
            actual_sha256,
        });
    }
    Ok(bytes)
}
