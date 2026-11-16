use super::*;

#[test]
fn the_port_is_one_to_65535_and_defaults_to_8765() {
    let port = |value: Option<&str>| {
        OrbConfig::from_values(value.map(OsString::from), None).map(|config| config.port.get())
    };
    // (name, JARVIS_ORB_PORT, the port, or None when it is rejected)
    let cases = [
        ("unset", None, Some(8765)),
        ("empty counts as unset", Some(""), Some(8765)),
        ("the lowest port", Some("1"), Some(1)),
        ("a usual port", Some("9000"), Some(9000)),
        ("the highest port", Some("65535"), Some(65535)),
        ("surrounding spaces are ignored", Some(" 9000 "), Some(9000)),
        (
            "zero means any port, which the UI cannot know",
            Some("0"),
            None,
        ),
        ("one over the highest port", Some("65536"), None),
        ("negative", Some("-1"), None),
        ("not a number", Some("eight"), None),
    ];
    for (name, value, expected) in cases {
        match (port(value), expected) {
            (Ok(port), Some(expected)) => assert_eq!(port, expected, "{name}"),
            (
                Err(ConfigError::OrbPort {
                    value: reported, ..
                }),
                None,
            ) => {
                assert_eq!(Some(reported.as_str()), value, "{name}");
            }
            (other, expected) => panic!("{name}: {other:?} against {expected:?}"),
        }
    }
}

#[test]
fn a_port_that_is_not_text_is_rejected() {
    use std::os::unix::ffi::OsStringExt;

    let value = OsString::from_vec(vec![0xff, 0xfe]);
    let result = OrbConfig::from_values(Some(value), None);
    assert!(matches!(result, Err(ConfigError::OrbPort { .. })));
}

fn origins(value: Option<&str>) -> Result<Vec<String>, ConfigError> {
    OrbConfig::from_values(None, value.map(OsString::from)).map(|config| config.allowed_origins)
}

#[test]
fn the_allowed_origins_default_to_the_ui_server_on_localhost() {
    let expected = ["http://localhost:8766", "http://127.0.0.1:8766"];
    assert_eq!(origins(None).unwrap(), expected);
    assert_eq!(origins(Some("")).unwrap(), expected);
}

#[test]
fn the_allowed_origins_are_a_list_of_scheme_host_and_optional_port() {
    // (name, JARVIS_ORB_ALLOWED_ORIGINS, the origins kept)
    let cases: [(&str, &str, &[&str]); 7] = [
        ("one", "http://localhost:8766", &["http://localhost:8766"]),
        (
            "two, with spaces around them",
            "http://localhost:8766 , https://ui.local",
            &["http://localhost:8766", "https://ui.local"],
        ),
        (
            "scheme and host are lowercased",
            "HTTP://LocalHost:8766",
            &["http://localhost:8766"],
        ),
        ("no port", "http://localhost", &["http://localhost"]),
        ("the lowest port", "http://a:1", &["http://a:1"]),
        ("the highest port", "http://a:65535", &["http://a:65535"]),
        ("an IPv6 host", "http://[::1]:8766", &["http://[::1]:8766"]),
    ];
    for (name, value, expected) in cases {
        assert_eq!(origins(Some(value)).unwrap(), expected, "{name}");
    }
}

#[test]
fn a_malformed_allowed_origin_is_rejected_by_name() {
    // (name, JARVIS_ORB_ALLOWED_ORIGINS, the entry it is blamed on)
    let cases = [
        (
            "a trailing slash is a path",
            "http://localhost:8766/",
            "http://localhost:8766/",
        ),
        (
            "a path",
            "http://localhost:8766/ui",
            "http://localhost:8766/ui",
        ),
        ("a query", "http://localhost?x=1", "http://localhost?x=1"),
        ("no scheme", "localhost:8766", "localhost:8766"),
        (
            "a scheme that is not http",
            "ws://localhost:8766",
            "ws://localhost:8766",
        ),
        ("no host", "http://:8766", "http://:8766"),
        ("a user name", "http://me@localhost", "http://me@localhost"),
        ("port 0", "http://localhost:0", "http://localhost:0"),
        (
            "port 65536",
            "http://localhost:65536",
            "http://localhost:65536",
        ),
        (
            "a port that is not a number",
            "http://localhost:ui",
            "http://localhost:ui",
        ),
        ("an empty port", "http://localhost:", "http://localhost:"),
        ("an empty entry between two", "http://a,,http://b", ""),
        ("a trailing comma", "http://a,", ""),
        ("only spaces", " ", " "),
        (
            "one bad entry among good ones",
            "http://a,nope,http://b",
            "nope",
        ),
        ("junk after an IPv6 host", "http://[::1]x", "http://[::1]x"),
    ];
    for (name, value, blamed) in cases {
        match origins(Some(value)) {
            Err(ConfigError::OrbOrigin { value, .. }) => assert_eq!(value, blamed, "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }
}
