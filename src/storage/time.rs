use std::time::{SystemTime, UNIX_EPOCH};

use super::error::StorageError;

pub(super) fn unix_time_ms() -> Result<i64, StorageError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;
    Ok(duration.as_millis() as i64)
}
