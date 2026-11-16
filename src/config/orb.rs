use std::{env, ffi::OsString, num::NonZeroU16};

use thiserror::Error;

use super::ConfigError;

const PORT_VAR: &str = "JARVIS_ORB_PORT";
/// The WebSocket port when `JARVIS_ORB_PORT` is not set. It must match the port
/// constant in `ui/src/link/` (jarvis-ui ticket 04).
const DEFAULT_PORT: NonZeroU16 = NonZeroU16::new(8765).expect("8765 is not zero");
const ORIGINS_VAR: &str = "JARVIS_ORB_ALLOWED_ORIGINS";
/// The port of the static server that serves the UI to Chromium, used to build
/// the default allowed origins. It must match the port in the UI server's service
/// (jarvis-ui ticket 05).
const DEFAULT_UI_PORT: u16 = 8766;
/// The hosts the UI server may be reached by, for the default allowed origins.
const DEFAULT_UI_HOSTS: [&str; 2] = ["localhost", "127.0.0.1"];

/// Why an entry of `JARVIS_ORB_ALLOWED_ORIGINS` is not an origin.
#[derive(Debug, Error)]
pub enum OriginError {
    #[error("it is empty")]
    Empty,
    #[error("it must start with http:// or https://")]
    Scheme,
    #[error(
        "it needs a host, and nothing but a host and an optional port (no path, query or spaces)"
    )]
    Host,
    #[error("its port must be a number from 1 to 65535")]
    Port(#[source] std::num::ParseIntError),
}

/// Where Jarvis serves the orb screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrbConfig {
    /// The localhost port of the WebSocket the UI connects to: `JARVIS_ORB_PORT`,
    /// else 8765. Never 0: 0 would mean "any port", and the UI has to know which.
    pub port: NonZeroU16,
    /// The origins (`scheme://host[:port]`, lowercase) whose pages may use the
    /// WebSocket: `JARVIS_ORB_ALLOWED_ORIGINS`, a comma-separated list, else
    /// `http://localhost:8766` and `http://127.0.0.1:8766`. A page from any other
    /// origin, or a request with no origin, is refused, so that a web page open
    /// on the Pi cannot send taps.
    pub allowed_origins: Vec<String>,
}

impl OrbConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        Self::from_values(env::var_os(PORT_VAR), env::var_os(ORIGINS_VAR))
    }

    /// An empty value counts as unset.
    ///
    /// The port is `JARVIS_ORB_PORT` (1 to 65535, else [`ConfigError::OrbPort`])
    /// or the default. The origins are `JARVIS_ORB_ALLOWED_ORIGINS` (every entry
    /// valid, else [`ConfigError::OrbOrigin`]) or the default.
    fn from_values(port: Option<OsString>, origins: Option<OsString>) -> Result<Self, ConfigError> {
        let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
        let port = match set(port) {
            None => DEFAULT_PORT,
            Some(value) => {
                let value = value.to_string_lossy();
                value
                    .trim()
                    .parse()
                    .map_err(|source| ConfigError::OrbPort {
                        value: value.to_string(),
                        source,
                    })?
            }
        };
        let allowed_origins = match set(origins) {
            None => DEFAULT_UI_HOSTS
                .iter()
                .map(|host| format!("http://{host}:{DEFAULT_UI_PORT}"))
                .collect(),
            Some(value) => parse_origins(&value.to_string_lossy())?,
        };
        Ok(Self {
            port,
            allowed_origins,
        })
    }
}

/// A comma-separated list of origins. Every entry must be valid, so that a typo
/// does not silently leave the UI locked out.
fn parse_origins(list: &str) -> Result<Vec<String>, ConfigError> {
    list.split(',')
        .map(|entry| {
            parse_origin(entry).map_err(|source| ConfigError::OrbOrigin {
                value: entry.to_owned(),
                source,
            })
        })
        .collect()
}

/// `http` or `https`, `://`, a host, and optionally `:port`. Nothing else: a
/// trailing slash is a path and is refused, because a browser never sends one in
/// an `Origin`. Scheme and host are lowercased, as a browser sends them.
fn parse_origin(entry: &str) -> Result<String, OriginError> {
    let entry = entry.trim();
    if entry.is_empty() {
        return Err(OriginError::Empty);
    }
    let entry = entry.to_ascii_lowercase();
    let rest = entry
        .strip_prefix("http://")
        .or_else(|| entry.strip_prefix("https://"))
        .ok_or(OriginError::Scheme)?;
    // A bracketed IPv6 host keeps its own colons; the port follows the bracket.
    let (host, port) = match rest.strip_prefix('[') {
        Some(inner) => {
            let (address, after) = inner.split_once(']').ok_or(OriginError::Host)?;
            let port = match after {
                "" => None,
                after => Some(after.strip_prefix(':').ok_or(OriginError::Host)?),
            };
            (address, port)
        }
        None => match rest.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (rest, None),
        },
    };
    let host_is_plain = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    if !host_is_plain {
        return Err(OriginError::Host);
    }
    if let Some(port) = port {
        port.parse::<NonZeroU16>().map_err(OriginError::Port)?;
    }
    Ok(entry)
}

#[cfg(test)]
#[path = "../../tests/unit/config/orb_tests.rs"]
mod tests;
