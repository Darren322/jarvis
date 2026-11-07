use std::{collections::HashMap, env::VarError};

use super::*;

const URL: &str = "http://jarvis-ai.local:8082/v1";

fn parse(vars: &[(&'static str, &str)]) -> Result<Option<SpeechToTextConfig>, ConfigError> {
    let vars: HashMap<&'static str, String> = vars
        .iter()
        .map(|(name, value)| (*name, (*value).to_owned()))
        .collect();
    SpeechToTextConfig::from_lookup(|name| Ok(vars.get(name).cloned()))
}

#[test]
fn all_three_settings_make_a_config() {
    let config = parse(&[
        ("JARVIS_STT_BASE_URL", URL),
        ("JARVIS_STT_MODEL", "whisper-medium"),
        ("JARVIS_CAPTURE_DEVICE", "plughw:3,0"),
    ])
    .expect("complete settings are valid");

    assert_eq!(
        config,
        Some(SpeechToTextConfig {
            base_url: URL.to_owned(),
            model: "whisper-medium".to_owned(),
            capture_device: "plughw:3,0".to_owned(),
        })
    );
}

#[test]
fn no_settings_leave_speech_to_text_absent_and_empty_values_count_as_unset() {
    assert_eq!(parse(&[]).expect("absent is valid"), None);
    assert_eq!(
        parse(&[
            ("JARVIS_STT_BASE_URL", ""),
            ("JARVIS_STT_MODEL", ""),
            ("JARVIS_CAPTURE_DEVICE", ""),
        ])
        .expect("empty values are unset"),
        None
    );
}

/// Every combination of one or two settings, with the variable the error must name.
/// When several are missing, the first in URL, model, device order is named.
#[test]
fn a_partial_set_of_settings_names_the_missing_variable() {
    let url = ("JARVIS_STT_BASE_URL", URL);
    let model = ("JARVIS_STT_MODEL", "whisper-medium");
    let device = ("JARVIS_CAPTURE_DEVICE", "plughw:3,0");
    let cases: Vec<(Vec<(&'static str, &str)>, &str)> = vec![
        (vec![url], "JARVIS_STT_MODEL"),
        (vec![model], "JARVIS_STT_BASE_URL"),
        (vec![device], "JARVIS_STT_BASE_URL"),
        (vec![url, model], "JARVIS_CAPTURE_DEVICE"),
        (vec![url, device], "JARVIS_STT_MODEL"),
        (vec![model, device], "JARVIS_STT_BASE_URL"),
        (vec![url, ("JARVIS_STT_MODEL", "")], "JARVIS_STT_MODEL"),
    ];

    for (vars, missing) in cases {
        match parse(&vars) {
            Err(ConfigError::SpeechToTextIncomplete { name }) => {
                assert_eq!(name, missing, "settings {vars:?}");
            }
            other => panic!("settings {vars:?} gave {other:?}"),
        }
    }
}

#[test]
fn a_setting_that_is_not_unicode_is_an_error_naming_it() {
    let result = SpeechToTextConfig::from_lookup(|name| {
        if name == "JARVIS_STT_MODEL" {
            Err(VarError::NotUnicode("\u{fffd}".into()))
        } else {
            Ok(None)
        }
    });

    assert!(matches!(
        result,
        Err(ConfigError::Var {
            name: "JARVIS_STT_MODEL",
            ..
        })
    ));
}
