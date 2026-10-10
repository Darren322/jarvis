use std::{path::Path, time::Duration};

use tokio::io::AsyncReadExt;

use crate::storage::{MemoryRepository, SourceReceiptState};

use super::{MemoryError, types::ImportReceipt};

const MAX_IMPORT_BYTES: usize = 1024 * 1024;
const FILE_READ_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn import_file(
    repository: &MemoryRepository,
    path: &Path,
) -> Result<ImportReceipt, MemoryError> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if !metadata.file_type().is_file() {
        return Err(MemoryError::InvalidImport);
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("txt" | "md")) {
        return Err(MemoryError::InvalidImport);
    }

    let read = async {
        let file = tokio::fs::File::open(path).await?;
        if !file.metadata().await?.is_file() {
            return Err(MemoryError::InvalidImport);
        }
        let mut bytes = Vec::with_capacity(metadata.len().min(MAX_IMPORT_BYTES as u64) as usize);
        file.take((MAX_IMPORT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err(MemoryError::SourceTooLarge);
        }
        String::from_utf8(bytes).map_err(|_| MemoryError::InvalidImport)
    };
    let text = tokio::time::timeout(FILE_READ_TIMEOUT, read)
        .await
        .map_err(|_| MemoryError::SourceTooLarge)??;

    let canonical = tokio::fs::canonicalize(path).await?;
    let path_key = canonical.to_string_lossy().into_owned();
    let display_path = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or(MemoryError::InvalidImport)?
        .to_owned();
    let receipt = repository
        .import_source(path_key, display_path, text)
        .await?;
    Ok(ImportReceipt {
        source_id: receipt.source_id,
        revision_sha256: receipt.revision_sha256,
        state: match receipt.state {
            SourceReceiptState::Created => "created",
            SourceReceiptState::AlreadyCurrent => "already_current",
            SourceReceiptState::Reactivated => "reactivated",
            SourceReceiptState::SuppressedRevision => "suppressed_revision",
        }
        .to_owned(),
        parts: 1,
    })
}
