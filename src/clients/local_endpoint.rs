//! Keeps model traffic on the LAN. Every client that talks to a model server
//! validates its URL here and resolves names through `LocalEndpointResolver`.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub(crate) enum LocalEndpointError {
    #[error("local model endpoint URL is invalid")]
    InvalidUrl,
    #[error("local model endpoints must use HTTP or HTTPS")]
    UnsupportedScheme,
    #[error("local model endpoint URLs must not contain credentials")]
    CredentialsNotAllowed,
    #[error(
        "local model endpoint must use localhost, a .local name, or an explicit local IP address"
    )]
    NonLocalHost,
}

/// Parses local endpoints. A `.local` name is permitted only because the
/// request client validates its complete system-resolved address set before
/// opening an HTTP connection.
pub(super) fn validate_local_endpoint(value: &str) -> Result<reqwest::Url, LocalEndpointError> {
    let url = reqwest::Url::parse(value).map_err(|_| LocalEndpointError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(LocalEndpointError::UnsupportedScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(LocalEndpointError::CredentialsNotAllowed);
    }
    if url.fragment().is_some() {
        return Err(LocalEndpointError::InvalidUrl);
    }
    let host = url.host_str().ok_or(LocalEndpointError::InvalidUrl)?;
    if !is_local_host(host) {
        return Err(LocalEndpointError::NonLocalHost);
    }
    Ok(url)
}

fn is_local_host(host: &str) -> bool {
    // `Url::host_str()` retains brackets around IPv6 literals; strip only a
    // valid pair before parsing the address value.
    let ip_host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    match ip_host.parse::<IpAddr>() {
        Ok(address) => is_local_ip(address),
        Err(_) => host.eq_ignore_ascii_case("localhost") || is_local_mdns_name(host),
    }
}

fn is_local_mdns_name(host: &str) -> bool {
    // Accept ordinary mDNS hostnames and subdomains, with an optional DNS root
    // dot. URL parsing has already normalized international names to IDNA.
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.len() > 253 {
        return false;
    }
    let mut labels = host.split('.');
    let Some(suffix) = labels.next_back() else {
        return false;
    };
    let preceding_labels = labels.collect::<Vec<_>>();
    suffix.eq_ignore_ascii_case("local")
        && !preceding_labels.is_empty()
        && preceding_labels
            .into_iter()
            .map(str::as_bytes)
            .all(is_valid_dns_label)
}

fn is_valid_dns_label(label: &[u8]) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label[0].is_ascii_alphanumeric()
        && label[label.len() - 1].is_ascii_alphanumeric()
        && label
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

fn is_local_ipv4(address: Ipv4Addr) -> bool {
    address.is_loopback() || address.is_private() || address.is_link_local()
}

fn is_local_ipv6(address: Ipv6Addr) -> bool {
    if address.is_loopback() {
        return true;
    }
    let first = address.segments()[0];
    // fc00::/7 is unique-local and fe80::/10 is link-local.
    first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
}

fn is_local_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_local_ipv4(address),
        IpAddr::V6(address) => is_local_ipv6(address),
    }
}

pub(super) fn validate_resolved_addresses(addresses: Vec<SocketAddr>) -> Option<Vec<SocketAddr>> {
    (!addresses.is_empty() && addresses.iter().all(|address| is_local_ip(address.ip())))
        .then_some(addresses)
}

#[derive(Debug, thiserror::Error)]
enum LocalResolutionError {
    #[error("local DNS lookup failed")]
    Lookup(#[source] std::io::Error),
    #[error("local DNS resolution returned an empty or non-local address set")]
    UnsafeAddressSet,
}

/// Reqwest calls this for each connection attempt. Returning only the already
/// validated socket addresses prevents a second DNS lookup from changing the
/// destination between the security check and the connection.
#[derive(Debug, Clone, Copy)]
pub(super) struct LocalEndpointResolver;

impl Resolve for LocalEndpointResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|source| {
                    Box::new(LocalResolutionError::Lookup(source))
                        as Box<dyn std::error::Error + Send + Sync>
                })?
                .collect::<Vec<_>>();
            let addresses = validate_resolved_addresses(addresses).ok_or_else(|| {
                Box::new(LocalResolutionError::UnsafeAddressSet)
                    as Box<dyn std::error::Error + Send + Sync>
            })?;
            let addresses: Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

#[cfg(test)]
#[path = "../../tests/unit/clients/local_endpoint_tests.rs"]
mod tests;
