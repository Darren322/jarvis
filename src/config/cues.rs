use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::{configured_path, resolve_from};

const DIR_VAR: &str = "JARVIS_CUE_DIR";
const VOLUME_VAR: &str = "JARVIS_CUE_VOLUME";
/// Where the cue files are looked for, relative to the working directory. `data/`
/// is not in git: the files are someone else's audio and are copied in by hand.
const DEFAULT_DIR: &str = "data/sounds";
const DEFAULT_VOLUME: f32 = 0.7;

/// The short sounds that mark each state change.
#[derive(Clone, Debug, PartialEq)]
pub struct CueConfig {
    /// The folder holding one WAV file per cue. Absolute.
    pub dir: PathBuf,
    pub volume: CueVolume,
}

/// How loud the cues play, as a share of the files' own loudness.
#[derive(Clone, Debug, PartialEq)]
pub enum CueVolume {
    /// `JARVIS_CUE_VOLUME=0`: no cue is loaded or played.
    Off,
    /// Above 0, up to 1.
    Level(f32),
    /// The setting is not a number from 0 to 1. Cues stay off, and startup says why.
    Invalid(String),
}

impl CueConfig {
    pub(super) fn from_env(current_dir: &Path) -> Self {
        Self::from_values(env::var_os(DIR_VAR), env::var_os(VOLUME_VAR), current_dir)
    }

    /// An empty value counts as unset.
    fn from_values(dir: Option<OsString>, volume: Option<OsString>, current_dir: &Path) -> Self {
        let volume = match volume.filter(|value| !value.is_empty()) {
            None => CueVolume::Level(DEFAULT_VOLUME),
            Some(raw) => parse_volume(&raw.to_string_lossy()),
        };
        Self {
            dir: resolve_from(current_dir, configured_path(dir, DEFAULT_DIR)),
            volume,
        }
    }
}

fn parse_volume(raw: &str) -> CueVolume {
    match raw.trim().parse::<f32>() {
        Ok(0.0) => CueVolume::Off,
        Ok(level) if level > 0.0 && level <= 1.0 => CueVolume::Level(level),
        // Out of range, not finite, or not a number.
        Ok(_) | Err(_) => CueVolume::Invalid(raw.to_owned()),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/config/cues_tests.rs"]
mod tests;
