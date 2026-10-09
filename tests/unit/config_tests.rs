use std::{ffi::OsString, path::PathBuf};

use super::configured_path;

#[test]
fn memory_paths_default_when_unset_or_empty_and_preserve_custom_paths() {
    assert_eq!(
        configured_path(None, "data/embeddings"),
        PathBuf::from("data/embeddings")
    );
    assert_eq!(
        configured_path(Some(OsString::new()), "data/memory-index"),
        PathBuf::from("data/memory-index")
    );
    assert_eq!(
        configured_path(
            Some(OsString::from("/var/lib/jarvis/embeddings")),
            "data/embeddings"
        ),
        PathBuf::from("/var/lib/jarvis/embeddings")
    );
    assert_eq!(
        configured_path(Some(OsString::from("cache/index")), "data/memory-index"),
        PathBuf::from("cache/index")
    );
}
