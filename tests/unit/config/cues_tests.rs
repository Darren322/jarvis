use super::*;

fn os(value: &str) -> Option<OsString> {
    Some(OsString::from(value))
}

#[test]
fn the_cue_folder_defaults_to_data_sounds_and_is_always_absolute() {
    let cwd = Path::new("/home/darren/jarvis");
    // (name, JARVIS_CUE_DIR, expected)
    let cases = [
        ("unset", None, "/home/darren/jarvis/data/sounds"),
        (
            "empty counts as unset",
            os(""),
            "/home/darren/jarvis/data/sounds",
        ),
        ("a relative folder", os("cues"), "/home/darren/jarvis/cues"),
        ("an absolute folder", os("/srv/cues"), "/srv/cues"),
    ];

    for (name, dir, expected) in cases {
        let config = CueConfig::from_values(dir, None, cwd);
        assert_eq!(config.dir, PathBuf::from(expected), "{name}");
    }
}

#[test]
fn the_volume_is_a_number_from_0_to_1_and_0_turns_the_cues_off() {
    let invalid = |raw: &str| CueVolume::Invalid(raw.to_owned());
    // (JARVIS_CUE_VOLUME, expected)
    let cases = [
        (None, CueVolume::Level(DEFAULT_VOLUME)),
        (os(""), CueVolume::Level(DEFAULT_VOLUME)),
        (os("0"), CueVolume::Off),
        (os("0.0"), CueVolume::Off),
        // The smallest and largest levels that play, and the first values outside them.
        (os("0.01"), CueVolume::Level(0.01)),
        (os("1"), CueVolume::Level(1.0)),
        (os(" 0.5 "), CueVolume::Level(0.5)),
        (os("1.01"), invalid("1.01")),
        (os("-0.01"), invalid("-0.01")),
        (os("NaN"), invalid("NaN")),
        (os("inf"), invalid("inf")),
        (os("loud"), invalid("loud")),
    ];

    for (raw, expected) in cases {
        let config = CueConfig::from_values(None, raw.clone(), Path::new("/"));
        assert_eq!(config.volume, expected, "{raw:?}");
    }
}
