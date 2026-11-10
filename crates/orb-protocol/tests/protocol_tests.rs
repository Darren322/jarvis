use orb_protocol::{
    FromOrb, Level, LevelOutOfRange, LevelSource, MAX_LINE_BYTES, OrbState, ProtocolError, ToOrb,
};

fn level(value: f32) -> Level {
    Level::new(value).expect("test level is in range")
}

#[test]
fn every_message_to_the_orb_has_its_documented_line_and_round_trips() {
    let cases = [
        (
            ToOrb::State {
                state: OrbState::Idle,
            },
            r#"{"type":"state","state":"idle"}"#,
        ),
        (
            ToOrb::State {
                state: OrbState::Listening,
            },
            r#"{"type":"state","state":"listening"}"#,
        ),
        (
            ToOrb::State {
                state: OrbState::Transcribing,
            },
            r#"{"type":"state","state":"transcribing"}"#,
        ),
        (
            ToOrb::State {
                state: OrbState::Thinking,
            },
            r#"{"type":"state","state":"thinking"}"#,
        ),
        (
            ToOrb::State {
                state: OrbState::Speaking,
            },
            r#"{"type":"state","state":"speaking"}"#,
        ),
        (
            ToOrb::Level {
                level: level(0.25),
                source: LevelSource::Mic,
            },
            r#"{"type":"level","level":0.25,"source":"mic"}"#,
        ),
        (
            ToOrb::Level {
                level: level(0.5),
                source: LevelSource::Speech,
            },
            r#"{"type":"level","level":0.5,"source":"speech"}"#,
        ),
    ];

    for (message, wire) in cases {
        let line = message.encode().expect("encodes");
        assert_eq!(line, format!("{wire}\n"));
        assert_eq!(ToOrb::decode(&line).expect("decodes with newline"), message);
        assert_eq!(
            ToOrb::decode(wire).expect("decodes without newline"),
            message
        );
    }
}

#[test]
fn a_tap_has_its_documented_line_and_round_trips() {
    let line = FromOrb::Tap {}.encode().expect("encodes");
    assert_eq!(line, "{\"type\":\"tap\"}\n");
    assert_eq!(FromOrb::decode(&line).expect("decodes"), FromOrb::Tap {});
}

#[test]
fn a_level_is_accepted_from_0_to_1_inclusive_and_rejected_just_outside() {
    let accepted = [0.0, f32::MIN_POSITIVE, 0.5, 1.0_f32.next_down(), 1.0];
    for value in accepted {
        assert_eq!(Level::new(value).map(Level::value), Ok(value), "{value}");
    }

    let rejected = [
        0.0_f32.next_down(),
        1.0_f32.next_up(),
        -1.0,
        2.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ];
    for value in rejected {
        assert_eq!(Level::new(value), Err(LevelOutOfRange(value)), "{value}");
    }
    assert!(Level::new(f32::NAN).is_err());
}

#[test]
fn a_raw_measurement_is_clamped_into_range_and_nan_is_silence() {
    let cases = [
        (0.0_f32.next_down(), 0.0),
        (-3.0, 0.0),
        (0.0, 0.0),
        (0.4, 0.4),
        (1.0, 1.0),
        (1.0_f32.next_up(), 1.0),
        (f32::INFINITY, 1.0),
        (f32::NEG_INFINITY, 0.0),
        (f32::NAN, 0.0),
    ];
    for (raw, expected) in cases {
        assert_eq!(Level::clamped(raw).value(), expected, "{raw}");
    }
}

#[test]
fn a_decoded_level_must_be_in_range() {
    let line = |level: &str| format!(r#"{{"type":"level","level":{level},"source":"mic"}}"#);

    for accepted in ["0", "0.0", "1", "1.0"] {
        assert!(ToOrb::decode(&line(accepted)).is_ok(), "{accepted}");
    }
    // 1.0000001 is the first f32 above 1; -1e-45 is the first below 0.
    for rejected in ["1.0000001", "-1e-45", "-0.5", "2", "null", "\"0.5\""] {
        assert!(
            matches!(
                ToOrb::decode(&line(rejected)),
                Err(ProtocolError::Invalid(_))
            ),
            "{rejected}"
        );
    }
}

/// A valid tap padded with spaces (JSON allows them) to exactly `len` bytes.
fn padded_tap(len: usize) -> String {
    let (open, close) = (r#"{"type":"tap""#, "}");
    format!(
        "{open}{}{close}",
        " ".repeat(len - open.len() - close.len())
    )
}

#[test]
fn a_line_of_the_maximum_length_is_accepted_and_one_byte_more_is_rejected() {
    let longest = padded_tap(MAX_LINE_BYTES);
    assert_eq!(longest.len(), MAX_LINE_BYTES);
    assert_eq!(
        FromOrb::decode(&longest).expect("at the limit"),
        FromOrb::Tap {}
    );
    // The newline is not counted.
    assert!(FromOrb::decode(&format!("{longest}\n")).is_ok());

    let too_long = padded_tap(MAX_LINE_BYTES + 1);
    assert!(matches!(
        FromOrb::decode(&too_long),
        Err(ProtocolError::LineTooLong { len }) if len == MAX_LINE_BYTES + 1
    ));
    assert!(matches!(
        ToOrb::decode(&too_long),
        Err(ProtocolError::LineTooLong { .. })
    ));
}

#[test]
fn unknown_fields_variants_and_malformed_lines_are_rejected() {
    let to_orb = [
        r#"{"type":"state","state":"idle","extra":1}"#,
        r#"{"type":"level","level":0.5,"source":"mic","extra":1}"#,
        r#"{"type":"state","state":"dancing"}"#,
        r#"{"type":"level","level":0.5,"source":"radio"}"#,
        r#"{"type":"shout"}"#,
        r#"{"type":"tap"}"#,
        r#"{"type":"state"}"#,
        r#"{"state":"idle"}"#,
        r#"{"type":"state","state":"Idle"}"#,
        "",
        "not json",
        r#"{"type":"state","state":"idle"}{"type":"state","state":"idle"}"#,
    ];
    for line in to_orb {
        assert!(
            matches!(ToOrb::decode(line), Err(ProtocolError::Invalid(_))),
            "{line}"
        );
    }

    let from_orb = [
        r#"{"type":"tap","extra":1}"#,
        r#"{"type":"swipe"}"#,
        r#"{"type":"state","state":"idle"}"#,
        r#"{}"#,
        "",
        "tap",
    ];
    for line in from_orb {
        assert!(
            matches!(FromOrb::decode(line), Err(ProtocolError::Invalid(_))),
            "{line}"
        );
    }
}
