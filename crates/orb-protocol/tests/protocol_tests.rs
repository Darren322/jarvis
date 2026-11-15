use orb_protocol::{
    CpuPercent, CpuPercentOutOfRange, FromOrb, Level, LevelOutOfRange, LevelSource, MAX_LINE_BYTES,
    OrbState, PanelName, ProtocolError, ServiceHealth, ToOrb,
};

fn cpu(value: f32) -> CpuPercent {
    CpuPercent::new(value).expect("test cpu percent is in range")
}

fn stats(temperature_c: Option<f32>, llama: ServiceHealth, bge: ServiceHealth) -> ToOrb {
    ToOrb::Stats {
        cpu_percent: cpu(12.5),
        temperature_c,
        ram_used_bytes: 2,
        ram_total_bytes: 8,
        llama,
        bge,
    }
}

#[test]
fn every_panel_message_has_its_documented_line_and_round_trips() {
    let to_orb = [
        (
            ToOrb::ShowPanel {
                panel: PanelName::Stats,
            },
            r#"{"type":"show_panel","panel":"stats"}"#,
        ),
        (ToOrb::HidePanel {}, r#"{"type":"hide_panel"}"#),
        (
            stats(Some(55.5), ServiceHealth::Up, ServiceHealth::Down),
            r#"{"type":"stats","cpu_percent":12.5,"temperature_c":55.5,"ram_used_bytes":2,"ram_total_bytes":8,"llama":"up","bge":"down"}"#,
        ),
        (
            stats(None, ServiceHealth::Down, ServiceHealth::Up),
            r#"{"type":"stats","cpu_percent":12.5,"ram_used_bytes":2,"ram_total_bytes":8,"llama":"down","bge":"up"}"#,
        ),
    ];
    for (message, wire) in to_orb {
        let line = message.encode().expect("encodes");
        assert_eq!(line, format!("{wire}\n"));
        assert_eq!(line.matches('\n').count(), 1, "only the terminator: {wire}");
        assert_eq!(ToOrb::decode(&line).expect("decodes with newline"), message);
        assert_eq!(
            ToOrb::decode(wire).expect("decodes without newline"),
            message
        );
    }

    let line = FromOrb::PanelClosed {}.encode().expect("encodes");
    assert_eq!(line, "{\"type\":\"panel_closed\"}\n");
    assert_eq!(
        FromOrb::decode(&line).expect("decodes"),
        FromOrb::PanelClosed {}
    );
}

#[test]
fn a_missing_temperature_is_absent_on_the_wire_and_never_zero() {
    let line = stats(None, ServiceHealth::Up, ServiceHealth::Up)
        .encode()
        .expect("encodes");
    assert!(!line.contains("temperature_c"), "{line}");
    assert!(matches!(
        ToOrb::decode(&line),
        Ok(ToOrb::Stats {
            temperature_c: None,
            ..
        })
    ));
    // A real zero reading stays a zero.
    let zero = stats(Some(0.0), ServiceHealth::Up, ServiceHealth::Up);
    assert_eq!(
        ToOrb::decode(&zero.encode().expect("encodes")).expect("decodes"),
        zero
    );
}

#[test]
fn a_cpu_percent_is_accepted_from_0_to_100_inclusive_and_rejected_just_outside() {
    for value in [0.0, f32::MIN_POSITIVE, 50.0, 100.0_f32.next_down(), 100.0] {
        assert_eq!(CpuPercent::new(value).map(CpuPercent::value), Ok(value));
    }
    for value in [
        0.0_f32.next_down(),
        100.0_f32.next_up(),
        -1.0,
        101.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ] {
        assert_eq!(CpuPercent::new(value), Err(CpuPercentOutOfRange(value)));
    }
    assert!(CpuPercent::new(f32::NAN).is_err());

    let clamped = [
        (-3.0, 0.0),
        (42.0, 42.0),
        (100.0_f32.next_up(), 100.0),
        (f32::INFINITY, 100.0),
        (f32::NEG_INFINITY, 0.0),
        (f32::NAN, 0.0),
    ];
    for (raw, expected) in clamped {
        assert_eq!(CpuPercent::clamped(raw).value(), expected, "{raw}");
    }
}

#[test]
fn a_decoded_stats_message_must_have_a_valid_cpu_percent() {
    let line = |cpu: &str| {
        format!(
            r#"{{"type":"stats","cpu_percent":{cpu},"ram_used_bytes":2,"ram_total_bytes":8,"llama":"up","bge":"up"}}"#
        )
    };
    for accepted in ["0", "100", "100.0"] {
        assert!(ToOrb::decode(&line(accepted)).is_ok(), "{accepted}");
    }
    for rejected in ["100.00001", "-0.1", "null", "\"5\""] {
        assert!(
            matches!(
                ToOrb::decode(&line(rejected)),
                Err(ProtocolError::Invalid(_))
            ),
            "{rejected}"
        );
    }
}

#[test]
fn unknown_panels_extra_fields_and_missing_fields_are_rejected() {
    let to_orb = [
        r#"{"type":"show_panel","panel":"camera"}"#,
        r#"{"type":"show_panel","panel":"Stats"}"#,
        r#"{"type":"show_panel"}"#,
        r#"{"type":"show_panel","panel":"stats","extra":1}"#,
        r#"{"type":"hide_panel","extra":1}"#,
        r#"{"type":"stats","cpu_percent":1,"ram_used_bytes":2,"ram_total_bytes":8,"llama":"up","bge":"up","extra":1}"#,
        r#"{"type":"stats","cpu_percent":1,"ram_used_bytes":2,"ram_total_bytes":8,"llama":"sideways","bge":"up"}"#,
        r#"{"type":"stats","cpu_percent":1,"ram_used_bytes":2,"ram_total_bytes":8,"llama":"up"}"#,
        r#"{"type":"stats","cpu_percent":1,"ram_used_bytes":-1,"ram_total_bytes":8,"llama":"up","bge":"up"}"#,
        r#"{"type":"stats","cpu_percent":1,"ram_used_bytes":18446744073709551616,"ram_total_bytes":8,"llama":"up","bge":"up"}"#,
    ];
    for line in to_orb {
        assert!(
            matches!(ToOrb::decode(line), Err(ProtocolError::Invalid(_))),
            "{line}"
        );
    }
    for line in [
        r#"{"type":"panel_closed","extra":1}"#,
        r#"{"type":"hide_panel"}"#,
    ] {
        assert!(
            matches!(FromOrb::decode(line), Err(ProtocolError::Invalid(_))),
            "{line}"
        );
    }
}

/// The widest Stats line: both RAM fields `u64::MAX`, both services down, and the
/// widest `f32` text (9 significant digits, no exponent) for CPU and temperature.
fn widest_stats_line() -> String {
    format!(
        r#"{{"type":"stats","cpu_percent":0.00000123456789,"temperature_c":-1234567890000000.0,"ram_used_bytes":{max},"ram_total_bytes":{max},"llama":"down","bge":"down"}}"#,
        max = u64::MAX
    )
}

#[test]
fn the_line_limit_is_the_size_of_the_widest_stats_message() {
    let widest = widest_stats_line();
    assert_eq!(widest.len(), MAX_LINE_BYTES);
    assert!(ToOrb::decode(&widest).is_ok());
    // One byte more (a space is valid JSON) is rejected.
    assert!(matches!(
        ToOrb::decode(&format!("{widest} ")),
        Err(ProtocolError::LineTooLong { len }) if len == MAX_LINE_BYTES + 1
    ));

    // Encoding real values never exceeds the limit: sample f32 bit patterns
    // (deterministic generator) for both float fields.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // The high bits are the better mixed ones; the shift leaves 32 bits.
        u32::try_from(seed >> 32).expect("32 bits fit in u32")
    };
    for _ in 0..200_000 {
        let raw_cpu = f32::from_bits(next());
        let raw_temp = f32::from_bits(next());
        if !raw_temp.is_finite() || raw_cpu.is_nan() {
            continue;
        }
        let message = ToOrb::Stats {
            cpu_percent: CpuPercent::clamped(raw_cpu.abs()),
            temperature_c: Some(raw_temp),
            ram_used_bytes: u64::MAX,
            ram_total_bytes: u64::MAX,
            llama: ServiceHealth::Down,
            bge: ServiceHealth::Down,
        };
        let line = message.encode().expect("within the limit");
        assert!(line.len() <= MAX_LINE_BYTES + 1, "{line}");
    }
}

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
