//! The `/health` check of the model servers on the 16 GB Pi, for the stats
//! Dashboard. Only the check lives here; what to do with the answer does not.

use std::time::Duration;

use reqwest::{Url, redirect::Policy};

use super::local_endpoint::{LocalEndpointError, LocalEndpointResolver, validate_local_endpoint};

/// How long one check may take before the server counts as down. A LAN server
/// answers in milliseconds; this is also below the stats interval, so a check
/// started on one tick has its answer by the next. Jarvis stops waiting; the
/// server may still be working.
pub(crate) const HEALTH_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Debug, thiserror::Error)]
pub(crate) enum HealthError {
    #[error("health endpoint URL is not allowed")]
    InvalidEndpoint(#[from] LocalEndpointError),
    #[error("could not build the health HTTP client")]
    HttpClient(#[source] reqwest::Error),
}

/// Whether a server answered its health check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HealthStatus {
    Up,
    Down,
}

/// Probes `GET /health` at the root of one server. Cheap to clone: clones share
/// one connection pool.
#[derive(Clone, Debug)]
pub(crate) struct HealthCheck {
    http: reqwest::Client,
    url: Url,
}

impl HealthCheck {
    /// `base_url` is the server's configured URL (it may end in `/v1`); the check
    /// goes to `/health` at its root, like the speech-to-text probe.
    pub(crate) fn new(base_url: &str) -> Result<Self, HealthError> {
        let url = validate_local_endpoint(base_url)?
            .join("/health")
            .map_err(|_| LocalEndpointError::InvalidUrl)?;
        let http = reqwest::Client::builder()
            .retry(reqwest::retry::never())
            .no_proxy()
            .dns_resolver(LocalEndpointResolver)
            .redirect(Policy::none())
            .build()
            .map_err(HealthError::HttpClient)?;
        Ok(Self { http, url })
    }

    /// Up for a success status within [`HEALTH_TIMEOUT`]. A timeout, a connection
    /// failure and any other status are all Down. Dropping the future abandons
    /// the request; nothing is retried.
    pub(crate) async fn check(&self) -> HealthStatus {
        let request = self.http.get(self.url.clone()).send();
        match tokio::time::timeout(HEALTH_TIMEOUT, request).await {
            Ok(Ok(response)) if response.status().is_success() => HealthStatus::Up,
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => HealthStatus::Down,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/clients/health_tests.rs"]
mod tests;
