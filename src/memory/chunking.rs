use std::{fs::File, io::Read, path::Path};

use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

use crate::storage::{SourceChunk, SourcePart};

const TARGET_TOKENS: usize = 256;
const OVERLAP_TOKENS: usize = 32;
const MAX_CHUNK_BYTES: usize = 4096;
const TOKENIZER_BYTES: u64 = 711_396;
const TOKENIZER_SHA256: &str = "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66";

pub(crate) struct TokenChunker {
    tokenizer: Tokenizer,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum ChunkError {
    #[error("tokenizer artifact is missing or unreadable")]
    ArtifactIo(#[from] std::io::Error),
    #[error("local tokenizer does not match the pinned manifest hash")]
    ArtifactIntegrity,
    #[error("pinned tokenizer could not be loaded: {0}")]
    Tokenizer(String),
    #[error("tokenizer returned a non-UTF-8 source span")]
    InvalidSpan,
    #[error("one tokenizer token exceeds the stored source chunk byte limit")]
    OversizedToken,
}

impl TokenChunker {
    pub(crate) fn load(model_dir: &Path) -> Result<Self, ChunkError> {
        let path = model_dir.join("tokenizer.json");
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file() || metadata.len() != TOKENIZER_BYTES {
            return Err(ChunkError::ArtifactIntegrity);
        }
        let mut file = File::open(&path)?;
        let opened = file.metadata()?;
        if !opened.is_file() || opened.len() != TOKENIZER_BYTES {
            return Err(ChunkError::ArtifactIntegrity);
        }
        let mut bytes = Vec::with_capacity(TOKENIZER_BYTES as usize);
        file.by_ref()
            .take(TOKENIZER_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != TOKENIZER_BYTES {
            return Err(ChunkError::ArtifactIntegrity);
        }
        let actual_hash = format!("{:x}", Sha256::digest(&bytes));
        if actual_hash != TOKENIZER_SHA256 {
            return Err(ChunkError::ArtifactIntegrity);
        }
        let tokenizer = Tokenizer::from_bytes(bytes)
            .map_err(|error| ChunkError::Tokenizer(error.to_string()))?;
        Ok(Self { tokenizer })
    }

    pub(crate) fn chunk(&self, part: &SourcePart) -> Result<Vec<SourceChunk>, ChunkError> {
        let encoding = self
            .tokenizer
            .encode(part.text.as_str(), false)
            .map_err(|error| ChunkError::Tokenizer(error.to_string()))?;
        let offsets = encoding
            .get_offsets()
            .iter()
            .copied()
            .filter(|(start, end)| start < end)
            .collect::<Vec<_>>();
        if offsets.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunks = Vec::new();
        let mut token_start = 0;
        while token_start < offsets.len() {
            let mut token_end = (token_start + TARGET_TOKENS).min(offsets.len());
            let byte_start = offsets[token_start].0;
            let mut byte_end = offsets[token_end - 1].1;
            while byte_end.saturating_sub(byte_start) > MAX_CHUNK_BYTES
                && token_end > token_start + 1
            {
                token_end -= 1;
                byte_end = offsets[token_end - 1].1;
            }
            if byte_end.saturating_sub(byte_start) > MAX_CHUNK_BYTES {
                return Err(ChunkError::OversizedToken);
            }
            let text = part
                .text
                .get(byte_start..byte_end)
                .ok_or(ChunkError::InvalidSpan)?;
            if text.is_empty() {
                return Err(ChunkError::InvalidSpan);
            }
            chunks.push(SourceChunk {
                chunk_index: chunks.len() as u32,
                start_byte: u32::try_from(byte_start).map_err(|_| ChunkError::InvalidSpan)?,
                end_byte: u32::try_from(byte_end).map_err(|_| ChunkError::InvalidSpan)?,
                text: text.to_owned(),
            });
            if token_end == offsets.len() {
                break;
            }
            let next = token_end.saturating_sub(OVERLAP_TOKENS);
            token_start = next.max(token_start + 1);
        }
        Ok(chunks)
    }
}
