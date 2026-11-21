use jiff::{Timestamp, Zoned, tz::TimeZone};
use rig_agent::tool::Tool;
use rig_core::serde_json::{self, json};

use crate::tools::allowed_tool;
use crate::tools::current_time::{
    CurrentTimeArgs, CurrentTimeTool, TimePart, answer, format_current_time,
};

fn zoned(text: &str) -> Zoned {
    text.parse().expect("the test instant is valid")
}

fn unknown_zone() -> Zoned {
    Timestamp::UNIX_EPOCH.to_zoned(TimeZone::unknown())
}

#[test]
fn the_clock_reading_is_formatted_for_each_case() {
    // (instant, time, weekday, date, time_zone, utc_offset)
    let cases = [
        (
            zoned("2026-11-21T14:30:00[Asia/Singapore]"),
            (
                "2:30 PM",
                "Saturday",
                "21 November 2026",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2026-11-22T00:00:00[Asia/Singapore]"),
            (
                "12:00 AM",
                "Sunday",
                "22 November 2026",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2026-11-21T23:59:59[Asia/Singapore]"),
            (
                "11:59 PM",
                "Saturday",
                "21 November 2026",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2027-01-01T00:00:00[Asia/Singapore]"),
            (
                "12:00 AM",
                "Friday",
                "1 January 2027",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2026-12-31T23:59:00[Asia/Singapore]"),
            (
                "11:59 PM",
                "Thursday",
                "31 December 2026",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2026-11-21T09:05:00[America/St_Johns]"),
            (
                "9:05 AM",
                "Saturday",
                "21 November 2026",
                "America/St_Johns",
                "-03:30",
            ),
        ),
        (
            unknown_zone(),
            (
                "12:00 AM",
                "Thursday",
                "1 January 1970",
                "Etc/Unknown",
                "+00:00",
            ),
        ),
        // Either side of noon, where the 12-hour clock turns over.
        (
            zoned("2026-11-21T11:59:00[Asia/Singapore]"),
            (
                "11:59 AM",
                "Saturday",
                "21 November 2026",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2026-11-21T12:00:00[Asia/Singapore]"),
            (
                "12:00 PM",
                "Saturday",
                "21 November 2026",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2026-11-21T13:00:00[Asia/Singapore]"),
            (
                "1:00 PM",
                "Saturday",
                "21 November 2026",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
    ];

    for (now, (time, weekday, date, time_zone, utc_offset)) in cases {
        let got = format_current_time(&now);
        assert_eq!(
            (
                got.time.as_str(),
                got.weekday.as_str(),
                got.date.as_str(),
                got.time_zone.as_str(),
                got.utc_offset.as_str()
            ),
            (time, weekday, date, time_zone, utc_offset),
            "{now}"
        );
    }
}

#[test]
fn the_json_has_exactly_the_five_keys() {
    let value = serde_json::to_value(format_current_time(&zoned(
        "2026-11-21T14:30:00[Asia/Singapore]",
    )))
    .expect("the reading serializes");

    assert_eq!(
        value,
        json!({
            "time": "2:30 PM",
            "weekday": "Saturday",
            "date": "21 November 2026",
            "time_zone": "Asia/Singapore",
            "utc_offset": "+08:00"
        })
    );
}

#[test]
fn current_time_tool_has_expected_contract() {
    assert_eq!(CurrentTimeTool::NAME, "current_time");
    assert_eq!(
        CurrentTimeTool.parameters(),
        json!({
            "type": "object",
            "properties": {
                "part": {
                    "type": "string",
                    "enum": ["time", "date", "all"],
                    "description": "What the question asks for."
                }
            },
            "required": ["part"],
            "additionalProperties": false
        })
    );
}

#[test]
fn each_part_returns_only_what_it_names() {
    let at = || format_current_time(&zoned("2026-11-21T19:38:00[Asia/Singapore]"));
    let cases = [
        (TimePart::Time, json!({ "time": "7:38 PM" })),
        (
            TimePart::Date,
            json!({ "weekday": "Saturday", "date": "21 November 2026" }),
        ),
        (
            TimePart::All,
            json!({
                "time": "7:38 PM",
                "weekday": "Saturday",
                "date": "21 November 2026",
                "time_zone": "Asia/Singapore",
                "utc_offset": "+08:00"
            }),
        ),
    ];
    for (part, expected) in cases {
        assert_eq!(answer(at(), part), expected, "{part:?}");
    }
}

#[test]
fn the_part_is_read_from_the_arguments_and_defaults_to_all() {
    let cases = [
        (json!({ "part": "time" }), TimePart::Time),
        (json!({ "part": "date" }), TimePart::Date),
        (json!({ "part": "all" }), TimePart::All),
        (json!({}), TimePart::All),
    ];
    for (arguments, part) in cases {
        let args: CurrentTimeArgs =
            serde_json::from_value(arguments.clone()).expect("the arguments are valid");
        assert_eq!(args.part, part, "{arguments}");
    }
}

#[test]
fn the_description_covers_the_questions_and_the_unknown_zone() {
    let description = CurrentTimeTool.description();

    for word in [
        "time",
        "date",
        "day of the week",
        "never guess",
        "Set `part`",
        "\"time\" for the time of day",
        "Say only what it returns",
        "Etc/Unknown",
    ] {
        assert!(description.contains(word), "{word}");
    }
}

#[test]
fn the_rule_accepts_the_parts_and_rejects_anything_else() {
    let rule = allowed_tool("current_time").expect("current_time is allowed");

    for accepted in [
        json!({"part": "time"}),
        json!({"part": "date"}),
        json!({"part": "all"}),
        json!({}),
    ] {
        assert!(rule.accepts_arguments(&accepted), "{accepted}");
    }
    for rejected in [
        json!({"part": "hour"}),
        json!({"part": "Time"}),
        json!({"part": "time", "zone": "UTC"}),
        json!({"zone": "UTC"}),
        json!(null),
    ] {
        assert!(!rule.accepts_arguments(&rejected), "{rejected}");
    }
}
