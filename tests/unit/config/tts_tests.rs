use super::*;
use std::collections::HashMap;

const CHECKOUT: &str = "/jarvis";

fn parse(vars: &[(&'static str, &str)]) -> OptionalTtsConfig {
    let vars: HashMap<&'static str, String> = vars
        .iter()
        .map(|(name, value)| (*name, (*value).to_owned()))
        .collect();
    OptionalTtsConfig::from_lookup(Path::new(CHECKOUT), |name| Ok(vars.get(name).cloned()))
}

fn configured(vars: &[(&'static str, &str)]) -> TtsConfig {
    match parse(vars) {
        OptionalTtsConfig::Configured(config) => config,
        other => panic!("expected a configured engine, got {other:?}"),
    }
}

fn invalid(vars: &[(&'static str, &str)]) -> TtsConfigError {
    match parse(vars) {
        OptionalTtsConfig::Invalid(error) => error,
        other => panic!("expected an invalid configuration, got {other:?}"),
    }
}

#[test]
fn the_engine_defaults_to_supertonic_so_existing_env_files_keep_working() {
    let config = configured(&[
        ("JARVIS_TTS_MODEL_DIR", "models/supertonic"),
        ("JARVIS_TTS_PYTHON", "/venv/bin/python"),
    ]);

    assert_eq!(
        config.engine,
        TtsEngine::Supertonic {
            model_dir: PathBuf::from("/jarvis/models/supertonic")
        }
    );
    assert_eq!(
        config.worker_script,
        PathBuf::from("/jarvis/scripts/tts/supertonic_worker.py")
    );
    assert_eq!(config.python, PathBuf::from("/venv/bin/python"));
    assert_eq!(config.threads, 2);
    assert_eq!(config.audio_device, None);
}

#[test]
fn the_engine_setting_selects_supertonic_or_piper_and_rejects_anything_else() {
    let supertonic = [
        ("JARVIS_TTS_MODEL_DIR", "/models/supertonic"),
        ("JARVIS_TTS_PIPER_MODEL", "/models/piper/voice.onnx"),
        ("JARVIS_TTS_PYTHON", "/venv/bin/python"),
    ];
    let with_engine = |engine: &'static str| {
        let mut vars = supertonic.to_vec();
        vars.push(("JARVIS_TTS_ENGINE", engine));
        parse(&vars)
    };

    assert!(matches!(
        with_engine("supertonic"),
        OptionalTtsConfig::Configured(TtsConfig {
            engine: TtsEngine::Supertonic { .. },
            ..
        })
    ));
    assert!(matches!(
        with_engine("piper"),
        OptionalTtsConfig::Configured(TtsConfig {
            engine: TtsEngine::Piper { .. },
            ..
        })
    ));
    for unknown in ["Piper", "SUPERTONIC", "espeak", " piper"] {
        assert!(
            matches!(
                with_engine(unknown),
                OptionalTtsConfig::Invalid(TtsConfigError::UnknownEngine { ref value })
                    if value == unknown
            ),
            "{unknown:?} should be an unknown engine"
        );
    }
    assert!(matches!(
        with_engine(""),
        OptionalTtsConfig::Invalid(TtsConfigError::Empty {
            name: "JARVIS_TTS_ENGINE"
        })
    ));
}

#[test]
fn piper_takes_its_model_and_names_the_voice_after_the_file() {
    let config = configured(&[
        ("JARVIS_TTS_ENGINE", "piper"),
        (
            "JARVIS_TTS_PIPER_MODEL",
            "voices/en_GB-northern_english_male-medium.onnx",
        ),
        ("JARVIS_TTS_PYTHON", "/venv/bin/python"),
        ("JARVIS_TTS_THREADS", "4"),
    ]);

    assert_eq!(
        config.engine,
        TtsEngine::Piper {
            model: PathBuf::from("/jarvis/voices/en_GB-northern_english_male-medium.onnx"),
            voice: "en_GB-northern_english_male-medium".to_owned(),
        }
    );
    assert_eq!(
        config.worker_script,
        PathBuf::from("/jarvis/scripts/tts/piper_worker.py")
    );
    assert_eq!(config.threads, 4);
}

#[test]
fn an_explicit_worker_script_overrides_the_engine_default() {
    let config = configured(&[
        ("JARVIS_TTS_ENGINE", "piper"),
        ("JARVIS_TTS_PIPER_MODEL", "/models/voice.onnx"),
        ("JARVIS_TTS_PYTHON", "/venv/bin/python"),
        ("JARVIS_TTS_WORKER", "custom/worker.py"),
    ]);

    assert_eq!(
        config.worker_script,
        PathBuf::from("/jarvis/custom/worker.py")
    );
}

#[test]
fn a_piper_config_without_its_model_is_an_error_not_disabled_speech() {
    let vars = [
        ("JARVIS_TTS_ENGINE", "piper"),
        ("JARVIS_TTS_PYTHON", "/venv/bin/python"),
        // The Supertonic model directory does not stand in for the Piper model.
        ("JARVIS_TTS_MODEL_DIR", "/models/supertonic"),
    ];

    assert_eq!(
        invalid(&vars),
        TtsConfigError::Missing {
            name: "JARVIS_TTS_PIPER_MODEL"
        }
    );
    let mut empty = vars.to_vec();
    empty.push(("JARVIS_TTS_PIPER_MODEL", ""));
    assert_eq!(
        invalid(&empty),
        TtsConfigError::Empty {
            name: "JARVIS_TTS_PIPER_MODEL"
        }
    );
}

#[test]
fn a_piper_model_must_name_a_file() {
    for model in ["/", ".onnx", "/models/.onnx"] {
        assert_eq!(
            invalid(&[
                ("JARVIS_TTS_ENGINE", "piper"),
                ("JARVIS_TTS_PIPER_MODEL", model),
                ("JARVIS_TTS_PYTHON", "/venv/bin/python"),
            ]),
            TtsConfigError::PiperModelNotAFile,
            "{model:?}"
        );
    }
}

#[test]
fn supertonic_stays_disabled_without_a_model_directory() {
    assert!(matches!(
        parse(&[("JARVIS_TTS_PYTHON", "/venv/bin/python")]),
        OptionalTtsConfig::Disabled
    ));
    // A Piper model alone does not enable the default engine.
    assert!(matches!(
        parse(&[("JARVIS_TTS_PIPER_MODEL", "/models/voice.onnx")]),
        OptionalTtsConfig::Disabled
    ));
    assert_eq!(
        invalid(&[("JARVIS_TTS_MODEL_DIR", "")]),
        TtsConfigError::Empty {
            name: "JARVIS_TTS_MODEL_DIR"
        }
    );
}

#[test]
fn the_python_interpreter_is_required_and_absolute_for_either_engine() {
    for engine_vars in [
        vec![
            ("JARVIS_TTS_ENGINE", "supertonic"),
            ("JARVIS_TTS_MODEL_DIR", "/models/supertonic"),
        ],
        vec![
            ("JARVIS_TTS_ENGINE", "piper"),
            ("JARVIS_TTS_PIPER_MODEL", "/models/voice.onnx"),
        ],
    ] {
        let with_python = |python: &'static str| {
            let mut vars = engine_vars.clone();
            vars.push(("JARVIS_TTS_PYTHON", python));
            vars
        };

        assert_eq!(
            invalid(&engine_vars),
            TtsConfigError::Missing {
                name: "JARVIS_TTS_PYTHON"
            }
        );
        assert_eq!(
            invalid(&with_python("")),
            TtsConfigError::Empty {
                name: "JARVIS_TTS_PYTHON"
            }
        );
        assert_eq!(
            invalid(&with_python("python3")),
            TtsConfigError::NotAbsolute {
                name: "JARVIS_TTS_PYTHON"
            }
        );
    }
}

#[test]
fn threads_run_from_1_to_4_and_default_to_2() {
    let with_threads = |threads: &str| {
        parse(&[
            ("JARVIS_TTS_MODEL_DIR", "/models/supertonic"),
            ("JARVIS_TTS_PYTHON", "/venv/bin/python"),
            ("JARVIS_TTS_THREADS", threads),
        ])
    };

    for (threads, expected) in [("1", Some(1)), ("4", Some(4)), ("0", None), ("5", None)] {
        match (with_threads(threads), expected) {
            (OptionalTtsConfig::Configured(config), Some(expected)) => {
                assert_eq!(config.threads, expected);
            }
            (OptionalTtsConfig::Invalid(TtsConfigError::Threads), None) => {}
            (other, _) => panic!("{threads:?} gave {other:?}"),
        }
    }
    for not_a_number in ["", "two", "-1", "2.5"] {
        assert!(
            matches!(
                with_threads(not_a_number),
                OptionalTtsConfig::Invalid(TtsConfigError::Threads)
            ),
            "{not_a_number:?}"
        );
    }
}

#[test]
fn a_setting_that_is_not_unicode_is_reported_by_name() {
    let config = OptionalTtsConfig::from_lookup(Path::new(CHECKOUT), |name| {
        if name == "JARVIS_TTS_ENGINE" {
            Err(env::VarError::NotUnicode(std::ffi::OsString::new()))
        } else {
            Ok(None)
        }
    });

    assert!(matches!(
        config,
        OptionalTtsConfig::Invalid(TtsConfigError::NotUnicode {
            name: "JARVIS_TTS_ENGINE"
        })
    ));
}

#[test]
fn the_audio_device_is_optional_but_not_empty() {
    let base = [
        ("JARVIS_TTS_MODEL_DIR", "/models/supertonic"),
        ("JARVIS_TTS_PYTHON", "/venv/bin/python"),
    ];
    let mut with_device = base.to_vec();
    with_device.push(("JARVIS_AUDIO_DEVICE", "alsa:usb"));
    assert_eq!(
        configured(&with_device).audio_device.as_deref(),
        Some("alsa:usb")
    );

    let mut empty_device = base.to_vec();
    empty_device.push(("JARVIS_AUDIO_DEVICE", ""));
    assert_eq!(
        invalid(&empty_device),
        TtsConfigError::Empty {
            name: "JARVIS_AUDIO_DEVICE"
        }
    );
}
