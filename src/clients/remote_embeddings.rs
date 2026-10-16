//! Client for the BGE embedding server (`llama-server --embedding`) on the
//! 16 GB Pi. See `docs/TWO_PI_BGE_DEPLOYMENT.md` for how it is started.

use std::time::Duration;

use reqwest::{Url, redirect::Policy};
use serde::{Deserialize, Serialize};

use super::local_endpoint::{LocalEndpointError, LocalEndpointResolver, validate_local_endpoint};
use crate::config::EmbeddingConfig;

/// Identifies the vector recipe. Changing any part re-embeds every memory.
/// The server cannot prove which file it loaded, so this records the
/// deployment Jarvis expects rather than one it verified.
pub(crate) const MODEL_FINGERPRINT: &str = "bge-small-en-v1.5-gguf:q8_0:\
    d32f8c040ea3b516330eeb75b72bcc2d3a780ab7:\
    sha256=ec38e8da142596baa913124ae50550de284b6916bf59577ef2f0cb9660c2f514:\
    query-prefix-v1:raw-passage:cls:l2:first-512-tokens";

const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
/// BGE's trained window, including [CLS] and [SEP]. The server rejects longer
/// input instead of truncating it.
const MAX_INPUT_TOKENS: usize = 512;
/// [CLS] and [SEP], added by the server around every input.
const SPECIAL_TOKENS: usize = 2;
/// Must match storage's `EMBEDDING_DIMENSIONS`; storage rejects anything else.
const DIMENSIONS: usize = 384;
/// One 384-float vector is about 8 KiB of JSON; a tokenized input at most ~13 KiB.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_NORM_ERROR: f32 = 0.001;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Bounds one HTTP request. Jarvis stops waiting; the server may still finish.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, thiserror::Error)]
pub(crate) enum RemoteEmbeddingsError {
    #[error("embedding server URL is not allowed")]
    InvalidEndpoint(#[from] LocalEndpointError),
    #[error("embedding server request failed")]
    Http(#[from] reqwest::Error),
    #[error("embedding server response exceeded {MAX_RESPONSE_BYTES} bytes")]
    TooLarge,
    #[error("embedding server response was not the expected JSON")]
    Json(#[from] serde_json::Error),
    #[error("embedding server returned {0} vectors for one input")]
    WrongCount(usize),
    #[error("embedding had {0} dimensions, expected {DIMENSIONS}")]
    WrongDimensions(usize),
    #[error("embedding was not a finite unit vector")]
    NotNormalized,
}

/// Cheap to clone: `reqwest::Client` shares one connection pool.
#[derive(Clone, Debug)]
pub(crate) struct RemoteEmbeddings {
    http: reqwest::Client,
    model: String,
    embeddings_url: Url,
    tokenize_url: Url,
    health_url: Url,
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: Input<'a>,
    encoding_format: &'static str,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Input<'a> {
    Text(&'a str),
    Tokens(&'a [u32]),
}

#[derive(Serialize)]
struct TokenizeRequest<'a> {
    content: &'a str,
    add_special: bool,
}

#[derive(Deserialize)]
struct TokenizeResponse {
    tokens: Vec<u32>,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
}

impl RemoteEmbeddings {
    pub(crate) fn new(config: &EmbeddingConfig) -> Result<Self, RemoteEmbeddingsError> {
        let mut base_url = validate_local_endpoint(&config.base_url)?;
        // `Url::join` replaces the last segment unless the path ends in '/'.
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let join = |path| {
            base_url
                .join(path)
                .map_err(|_| LocalEndpointError::InvalidUrl)
        };
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .retry(reqwest::retry::never())
            .no_proxy()
            .dns_resolver(LocalEndpointResolver)
            .redirect(Policy::none())
            .build()?;
        Ok(Self {
            http,
            model: config.model.clone(),
            embeddings_url: join("embeddings")?,
            // llama.cpp serves these at the server root, not under /v1.
            tokenize_url: join("/tokenize")?,
            health_url: join("/health")?,
        })
    }

    pub(crate) async fn health_check(&self) -> Result<(), RemoteEmbeddingsError> {
        self.http
            .get(self.health_url.clone())
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub(crate) async fn embed_query(&self, query: &str) -> Result<Vec<f32>, RemoteEmbeddingsError> {
        self.embed(&format!("{QUERY_PREFIX}{query}")).await
    }

    pub(crate) async fn embed_document(
        &self,
        text: &str,
    ) -> Result<Vec<f32>, RemoteEmbeddingsError> {
        self.embed(text).await
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, RemoteEmbeddingsError> {
        // A WordPiece token covers at least one byte, so short text always
        // fits and skips the extra tokenize request.
        let tokens = if text.len() + SPECIAL_TOKENS <= MAX_INPUT_TOKENS {
            None
        } else {
            Some(self.first_tokens(text).await?)
        };
        let input = match &tokens {
            Some(tokens) => Input::Tokens(tokens),
            None => Input::Text(text),
        };
        let request = EmbeddingRequest {
            model: &self.model,
            input,
            encoding_format: "float",
        };
        let body = self.post(&self.embeddings_url, &request).await?;
        unit_vector(serde_json::from_slice(&body)?)
    }

    /// Tokenizes with [CLS] and [SEP] and keeps the first tokens that fit,
    /// ending with the closing [SEP]. BGE pools the [CLS] vector, so the
    /// start of the text matters most.
    async fn first_tokens(&self, text: &str) -> Result<Vec<u32>, RemoteEmbeddingsError> {
        let request = TokenizeRequest {
            content: text,
            add_special: true,
        };
        let body = self.post(&self.tokenize_url, &request).await?;
        let mut tokens = serde_json::from_slice::<TokenizeResponse>(&body)?.tokens;
        if tokens.len() > MAX_INPUT_TOKENS {
            let closing = tokens.pop();
            tokens.truncate(MAX_INPUT_TOKENS - 1);
            tokens.extend(closing);
        }
        Ok(tokens)
    }

    /// Reads the body in chunks so a misbehaving server cannot make Jarvis
    /// buffer an unbounded response.
    async fn post(
        &self,
        url: &Url,
        request: &impl Serialize,
    ) -> Result<Vec<u8>, RemoteEmbeddingsError> {
        let mut response = self
            .http
            .post(url.clone())
            .json(request)
            .send()
            .await?
            .error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(RemoteEmbeddingsError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn unit_vector(response: EmbeddingResponse) -> Result<Vec<f32>, RemoteEmbeddingsError> {
    let [item] = <[EmbeddingItem; 1]>::try_from(response.data)
        .map_err(|data| RemoteEmbeddingsError::WrongCount(data.len()))?;
    let vector = item.embedding;
    if vector.len() != DIMENSIONS {
        return Err(RemoteEmbeddingsError::WrongDimensions(vector.len()));
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !vector.iter().all(|value| value.is_finite()) || (norm - 1.0).abs() > MAX_NORM_ERROR {
        return Err(RemoteEmbeddingsError::NotNormalized);
    }
    Ok(vector)
}

#[cfg(test)]
#[path = "../../tests/unit/clients/remote_embeddings_tests.rs"]
mod tests;
