use std::{ffi::OsString, path::PathBuf};

use super::configured_path;

#[test]
fn paths_default_when_unset_or_empty_and_preserve_custom_paths() {
    assert_eq!(
        configured_path(None, "data/jarvis.sqlite3"),
        PathBuf::from("data/jarvis.sqlite3")
    );
    assert_eq!(
        configured_path(Some(OsString::new()), "data/jarvis.sqlite3"),
        PathBuf::from("data/jarvis.sqlite3")
    );
    assert_eq!(
        configured_path(
            Some(OsString::from("/var/lib/jarvis/jarvis.sqlite3")),
            "data/jarvis.sqlite3"
        ),
        PathBuf::from("/var/lib/jarvis/jarvis.sqlite3")
    );
    assert_eq!(
        configured_path(
            Some(OsString::from("archive/jarvis.sqlite3")),
            "data/jarvis.sqlite3"
        ),
        PathBuf::from("archive/jarvis.sqlite3")
    );
}
