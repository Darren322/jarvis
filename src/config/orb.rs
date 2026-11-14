use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::resolve_from;

const SOCKET_VAR: &str = "JARVIS_ORB_SOCKET";
const RUNTIME_DIR_VAR: &str = "XDG_RUNTIME_DIR";
const SOCKET_FILE_NAME: &str = "jarvis-orb.sock";
/// Where the socket goes without a runtime directory, relative to the working directory.
const FALLBACK_SOCKET_PATH: &str = "data/jarvis-orb.sock";

/// Where Jarvis serves the orb screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrbConfig {
    /// The Unix socket `jarvis-orb` connects to. Absolute.
    pub socket_path: PathBuf,
}

impl OrbConfig {
    pub(super) fn from_env(current_dir: &Path) -> Self {
        Self::from_values(
            env::var_os(SOCKET_VAR),
            env::var_os(RUNTIME_DIR_VAR),
            current_dir,
        )
    }

    /// Picks the socket path, first match wins:
    ///
    /// 1. `JARVIS_ORB_SOCKET`, resolved against the working directory.
    /// 2. `$XDG_RUNTIME_DIR/jarvis-orb.sock`: a per-user directory in memory, mode
    ///    0700, emptied at logout and reboot, so no other user can reach the socket
    ///    and no stale file survives a restart. A relative value is not a valid
    ///    runtime directory and is skipped.
    /// 3. `data/jarvis-orb.sock` under the working directory, for a session with no
    ///    runtime directory (a Mac, some non-login shells).
    ///
    /// An empty value counts as unset.
    fn from_values(
        socket: Option<OsString>,
        runtime_dir: Option<OsString>,
        current_dir: &Path,
    ) -> Self {
        let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
        let socket_path = if let Some(socket) = set(socket) {
            resolve_from(current_dir, PathBuf::from(socket))
        } else if let Some(runtime_dir) = set(runtime_dir)
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
        {
            runtime_dir.join(SOCKET_FILE_NAME)
        } else {
            current_dir.join(FALLBACK_SOCKET_PATH)
        };
        Self { socket_path }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/config/orb_tests.rs"]
mod tests;
