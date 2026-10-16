use std::net::SocketAddr;

use super::{LocalEndpointError, validate_local_endpoint, validate_resolved_addresses};

/// Each pair sits on either side of an allowed range or a hostname rule.
#[rustfmt::skip]
#[test]
fn local_endpoint_guard_accepts_exactly_the_local_ranges() {
    let label_63 = "a".repeat(63);
    let label_64 = "a".repeat(64);
    let host_253 = format!("{}.local", vec!["a".repeat(61); 4].join("."));
    let host_254 = format!("a{host_253}");
    assert_eq!((host_253.len(), host_254.len()), (253, 254));

    let cases: Vec<(String, bool)> = vec![
        // IPv4: loopback, the three private ranges, and link-local, each
        // checked at its edge.
        ("127.0.0.1".into(), true),       ("128.0.0.1".into(), false),
        ("10.255.255.255".into(), true),  ("11.0.0.0".into(), false),
        ("172.16.0.0".into(), true),      ("172.15.255.255".into(), false),
        ("172.31.255.255".into(), true),  ("172.32.0.0".into(), false),
        ("192.168.0.1".into(), true),     ("192.169.0.1".into(), false),
        ("169.254.0.1".into(), true),     ("169.255.0.1".into(), false),
        // IPv6: loopback, unique-local fc00::/7, link-local fe80::/10.
        ("[::1]".into(), true),           ("[::2]".into(), false),
        ("[fc00::1]".into(), true),       ("[fbff::1]".into(), false),
        ("[fdff::1]".into(), true),       ("[fe00::1]".into(), false),
        ("[fe80::1]".into(), true),       ("[fec0::1]".into(), false),
        // Names: localhost and well-formed .local names only.
        ("localhost".into(), true),       ("api.example.com".into(), false),
        ("jarvis-ai.local".into(), true), ("jarvis-ai.local.example".into(), false),
        ("UPPER.Local.".into(), true),    (".local".into(), false),
        (format!("{label_63}.local"), true), (format!("{label_64}.local"), false),
        (host_253, true),                 (host_254, false),
        ("a-b.local".into(), true),       ("-ab.local".into(), false),
        ("ab.local".into(), true),        ("ab-.local".into(), false),
        ("ab.local".into(), true),        ("a_b.local".into(), false),
    ];

    for (host, accepted) in cases {
        let result = validate_local_endpoint(&format!("http://{host}:8080/v1"));
        assert_eq!(result.is_ok(), accepted, "{host}: {result:?}");
    }
}

#[test]
fn endpoint_urls_reject_credentials_and_non_http_schemes() {
    assert!(validate_local_endpoint("https://127.0.0.1:1234/v1").is_ok());
    assert_eq!(
        validate_local_endpoint("http://user:secret@127.0.0.1:1234/v1"),
        Err(LocalEndpointError::CredentialsNotAllowed)
    );
    assert_eq!(
        validate_local_endpoint("file:///tmp/model"),
        Err(LocalEndpointError::UnsupportedScheme)
    );
}

#[test]
fn dns_results_must_be_nonempty_and_every_address_local() {
    let local: SocketAddr = "192.168.1.20:0".parse().expect("valid socket");
    let public: SocketAddr = "8.8.8.8:0".parse().expect("valid socket");

    assert_eq!(validate_resolved_addresses(vec![local]), Some(vec![local]));
    assert_eq!(validate_resolved_addresses(Vec::new()), None);
    // One public address among local ones poisons the whole set.
    assert_eq!(validate_resolved_addresses(vec![local, public]), None);
}
