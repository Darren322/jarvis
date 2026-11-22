//! The timer messages (timers 05): the screen contract of D14.
//!
//! `VALID_TO_ORB`, `INVALID_TO_ORB`, `VALID_FROM_ORB` and `INVALID_FROM_ORB` are
//! the same tables, line for line, as in `ui/tests/link/timers.test.ts`.

use orb_protocol::{
    FromOrb, MAX_LINE_BYTES, MAX_TIMER_LABEL_CHARS, MAX_TIMERS, MAX_TO_ORB_LINE_BYTES,
    ProtocolError, Timer, TimerLabelTooLong, TimerState, ToOrb,
};

fn timer(id: u64, label: Option<&str>, state: TimerState) -> Timer {
    Timer::new(id, label.map(str::to_owned), 1_763_830_800_000, state)
        .expect("test timer is within the limits")
}

#[test]
fn the_timers_message_has_its_documented_line_and_round_trips() {
    let message = ToOrb::Timers {
        timers: vec![
            timer(3, Some("Time to sleep"), TimerState::Running),
            timer(4, None, TimerState::Pending),
            timer(5, Some("Tea"), TimerState::Ended),
        ],
    };
    let line = message.encode().expect("encodes");
    assert_eq!(
        line,
        concat!(
            r#"{"type":"timers","timers":["#,
            r#"{"id":3,"label":"Time to sleep","ends_at_ms":1763830800000,"state":"running"},"#,
            r#"{"id":4,"ends_at_ms":1763830800000,"state":"pending"},"#,
            r#"{"id":5,"label":"Tea","ends_at_ms":1763830800000,"state":"ended"}"#,
            "]}\n"
        )
    );
    assert_eq!(ToOrb::decode(&line).expect("decodes"), message);
}

#[test]
fn the_three_timer_answers_have_their_documented_line_and_round_trip() {
    let cases = [
        (
            FromOrb::CancelTimer { id: 3 },
            r#"{"type":"cancel_timer","id":3}"#,
        ),
        (
            FromOrb::ConfirmTimer { id: 3 },
            r#"{"type":"confirm_timer","id":3}"#,
        ),
        (
            FromOrb::RejectTimer { id: 3 },
            r#"{"type":"reject_timer","id":3}"#,
        ),
    ];
    for (message, text) in cases {
        assert_eq!(message.encode().expect("encodes"), format!("{text}\n"));
        assert_eq!(FromOrb::decode(text).expect("decodes"), message);
    }
}

fn label_of(chars: usize, unit: &str) -> String {
    unit.repeat(chars)
}

fn one_timer(label: &str) -> String {
    format!(
        r#"{{"type":"timers","timers":[{{"id":1,"label":"{label}","ends_at_ms":2,"state":"running"}}]}}"#
    )
}

#[test]
fn every_valid_timers_line_decodes_and_every_invalid_one_is_rejected() {
    let label_60_ascii = one_timer(&label_of(60, "a"));
    let label_60_emoji = one_timer(&label_of(60, "\u{1f600}"));
    let label_61_ascii = one_timer(&label_of(61, "a"));
    let label_61_emoji = one_timer(&label_of(61, "\u{1f600}"));
    let valid: Vec<&str> = vec![
        // VALID_TO_ORB
        r#"{"type":"timers","timers":[]}"#,
        r#"{"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":1763830800000,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":0,"ends_at_ms":0,"state":"pending"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"label":"","ends_at_ms":2,"state":"ended"}]}"#,
        r#"{"type":"timers","timers":[{"id":9007199254740991,"ends_at_ms":9007199254740991,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"state":"running","ends_at_ms":2,"label":"Reordered","id":1}]}"#,
        &label_60_ascii,
        &label_60_emoji,
    ];
    for line in valid {
        assert!(ToOrb::decode(line).is_ok(), "should decode: {line}");
    }

    let invalid: Vec<&str> = vec![
        // INVALID_TO_ORB
        r#"{"type":"timers"}"#,
        r#"{"type":"timers","timers":null}"#,
        r#"{"type":"timers","timers":{}}"#,
        r#"{"type":"timers","timers":[],"extra":1}"#,
        r#"{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":"cancelled"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":"Running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":null}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":"running","extra":1}]}"#,
        r#"{"type":"timers","timers":[{"ends_at_ms":2,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"ends_at_ms":2}]}"#,
        r#"{"type":"timers","timers":[{"id":-1,"ends_at_ms":2,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1.5,"ends_at_ms":2,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":"1","ends_at_ms":2,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"ends_at_ms":-2,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"ends_at_ms":1.5,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"ends_at_ms":"2","state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"label":null,"ends_at_ms":2,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[{"id":1,"label":3,"ends_at_ms":2,"state":"running"}]}"#,
        r#"{"type":"timers","timers":[3]}"#,
        &label_61_ascii,
        &label_61_emoji,
    ];
    for line in invalid {
        assert!(ToOrb::decode(line).is_err(), "should be rejected: {line}");
    }

    let valid_from_orb = [
        // VALID_FROM_ORB
        r#"{"type":"cancel_timer","id":3}"#,
        r#"{"type":"confirm_timer","id":0}"#,
        r#"{"type":"reject_timer","id":9007199254740991}"#,
        r#"{"id":3,"type":"cancel_timer"}"#,
    ];
    for line in valid_from_orb {
        assert!(FromOrb::decode(line).is_ok(), "should decode: {line}");
    }
    let invalid_from_orb = [
        // INVALID_FROM_ORB
        r#"{"type":"cancel_timer"}"#,
        r#"{"type":"confirm_timer","id":-1}"#,
        r#"{"type":"reject_timer","id":1.5}"#,
        r#"{"type":"cancel_timer","id":"3"}"#,
        r#"{"type":"cancel_timer","id":null}"#,
        r#"{"type":"confirm_timer","id":3,"extra":1}"#,
        r#"{"type":"pause_timer","id":3}"#,
        r#"{"type":"tap","id":3}"#,
    ];
    for line in invalid_from_orb {
        assert!(FromOrb::decode(line).is_err(), "should be rejected: {line}");
    }
}

#[test]
fn a_timer_label_accepts_60_characters_and_rejects_the_61st_by_characters_not_bytes() {
    assert_eq!(MAX_TIMER_LABEL_CHARS, 60);
    let emoji_60 = label_of(60, "\u{1f600}");
    assert!(emoji_60.len() > MAX_TIMER_LABEL_CHARS);
    assert!(Timer::new(1, Some(emoji_60), 2, TimerState::Running).is_ok());
    assert_eq!(
        Timer::new(1, Some(label_of(61, "a")), 2, TimerState::Running),
        Err(TimerLabelTooLong { chars: 61 })
    );
}

/// The widest timers message: the most timers, each with the largest id and
/// time, the longest state and a label of the most characters written with the
/// character JSON writes widest (`\u0001`, six bytes).
fn widest_timers() -> ToOrb {
    let widest = Timer::new(
        u64::MAX,
        Some(label_of(MAX_TIMER_LABEL_CHARS, "\u{1}")),
        u64::MAX,
        TimerState::Pending,
    )
    .expect("within the limits");
    ToOrb::Timers {
        timers: vec![widest; MAX_TIMERS],
    }
}

#[test]
fn five_timers_with_the_longest_labels_fit_the_line_limit_to_the_orb() {
    let line = widest_timers().encode().expect("the widest timers encode");
    let widest = line.trim_end_matches('\n');
    assert!(widest.len() <= MAX_TO_ORB_LINE_BYTES);
    assert!(ToOrb::decode(widest).is_ok());
    // The limit is still set by the wider built Panel: a timers line is
    // narrower, and the constant is the larger of the two.
    assert!(widest.len() < MAX_TO_ORB_LINE_BYTES);
}

#[test]
fn the_three_timer_answers_fit_the_inbound_line_limit_at_the_largest_id() {
    for message in [
        FromOrb::CancelTimer { id: u64::MAX },
        FromOrb::ConfirmTimer { id: u64::MAX },
        FromOrb::RejectTimer { id: u64::MAX },
    ] {
        let line = message.encode().expect("fits at the largest id");
        let text = line.trim_end_matches('\n');
        assert!(text.len() <= MAX_LINE_BYTES);
        assert_eq!(FromOrb::decode(text).expect("decodes"), message);
    }
}

#[test]
fn a_timers_line_over_the_limit_is_rejected_before_parsing() {
    let line = widest_timers().encode().expect("encodes");
    let padded = format!("{}{}", line.trim_end(), " ".repeat(MAX_TO_ORB_LINE_BYTES));
    assert!(matches!(
        ToOrb::decode(&padded),
        Err(ProtocolError::LineTooLong { .. })
    ));
}
