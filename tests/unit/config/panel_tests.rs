use super::*;

fn os(value: &str) -> Option<OsString> {
    Some(OsString::from(value))
}

#[test]
fn the_dashboard_timeout_defaults_to_thirty_seconds() {
    for unset in [None, os("")] {
        let config = PanelConfig::from_values(unset).expect("an unset timeout is valid");
        assert_eq!(config.dashboard_timeout, Duration::from_secs(30));
    }
}

#[test]
fn the_dashboard_timeout_accepts_one_second_and_rejects_zero() {
    // (setting, expected seconds or None when rejected)
    let cases = [
        ("1", Some(1)),
        ("0", None),
        ("45", Some(45)),
        (" 2 ", Some(2)),
    ];
    for (setting, expected) in cases {
        let result = PanelConfig::from_values(os(setting));
        match expected {
            Some(seconds) => assert_eq!(
                result.expect("the setting is valid").dashboard_timeout,
                Duration::from_secs(seconds),
                "{setting:?}"
            ),
            None => assert!(
                matches!(
                    result,
                    Err(ConfigError::PanelTimeout {
                        source: PanelTimeoutError::Zero,
                        ..
                    })
                ),
                "{setting:?}"
            ),
        }
    }
}

#[test]
fn a_dashboard_timeout_that_is_not_a_whole_number_is_rejected() {
    for setting in ["-1", "1.5", "soon", "30s"] {
        assert!(
            matches!(
                PanelConfig::from_values(os(setting)),
                Err(ConfigError::PanelTimeout {
                    source: PanelTimeoutError::NotANumber(_),
                    ..
                })
            ),
            "{setting:?}"
        );
    }
}
