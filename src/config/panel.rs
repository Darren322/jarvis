use std::{env, ffi::OsString, num::ParseIntError, time::Duration};

use thiserror::Error;

use super::ConfigError;
use crate::features::panel::PANEL_TIMEOUT;

const TIMEOUT_VAR: &str = "JARVIS_PANEL_TIMEOUT_SECONDS";

/// Why `JARVIS_PANEL_TIMEOUT_SECONDS` is not a timeout.
#[derive(Debug, Error)]
pub enum PanelTimeoutError {
    #[error("it must be a whole number of seconds")]
    NotANumber(#[source] ParseIntError),
    #[error("it must be at least 1: a panel that hides at once cannot be seen")]
    Zero,
}

/// How the panels on the orb screen behave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelConfig {
    /// How long the stats Dashboard stays up when nobody closes it:
    /// `JARVIS_PANEL_TIMEOUT_SECONDS`, else 30 seconds. Never zero.
    pub dashboard_timeout: Duration,
}

impl PanelConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        Self::from_values(env::var_os(TIMEOUT_VAR))
    }

    /// An empty value counts as unset.
    fn from_values(timeout_seconds: Option<OsString>) -> Result<Self, ConfigError> {
        let Some(raw) = timeout_seconds.filter(|value| !value.is_empty()) else {
            return Ok(Self {
                dashboard_timeout: PANEL_TIMEOUT,
            });
        };
        let raw = raw.to_string_lossy();
        let invalid = |source| ConfigError::PanelTimeout {
            value: raw.to_string(),
            source,
        };
        let seconds = raw
            .trim()
            .parse::<u64>()
            .map_err(|error| invalid(PanelTimeoutError::NotANumber(error)))?;
        if seconds == 0 {
            return Err(invalid(PanelTimeoutError::Zero));
        }
        Ok(Self {
            dashboard_timeout: Duration::from_secs(seconds),
        })
    }
}

#[cfg(test)]
#[path = "../../tests/unit/config/panel_tests.rs"]
mod tests;
