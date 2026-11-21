use jiff::{Timestamp, Zoned, tz::TimeZone};
use rig_agent::tool::Tool;
use rig_core::serde_json::{self, json};

use crate::tools::allowed_tool;
use crate::tools::current_time::{CurrentTimeTool, format_current_time};

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
                "14:30",
                "Saturday",
                "2026-11-21",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2026-11-22T00:00:00[Asia/Singapore]"),
            ("00:00", "Sunday", "2026-11-22", "Asia/Singapore", "+08:00"),
        ),
        (
            zoned("2026-11-21T23:59:59[Asia/Singapore]"),
            (
                "23:59",
                "Saturday",
                "2026-11-21",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2027-01-01T00:00:00[Asia/Singapore]"),
            ("00:00", "Friday", "2027-01-01", "Asia/Singapore", "+08:00"),
        ),
        (
            zoned("2026-12-31T23:59:00[Asia/Singapore]"),
            (
                "23:59",
                "Thursday",
                "2026-12-31",
                "Asia/Singapore",
                "+08:00",
            ),
        ),
        (
            zoned("2026-11-21T09:05:00[America/St_Johns]"),
            (
                "09:05",
                "Saturday",
                "2026-11-21",
                "America/St_Johns",
                "-03:30",
            ),
        ),
        (
            unknown_zone(),
            ("00:00", "Thursday", "1970-01-01", "Etc/Unknown", "+00:00"),
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
            "time": "14:30",
            "weekday": "Saturday",
            "date": "2026-11-21",
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
            "properties": {},
            "additionalProperties": false
        })
    );
}

#[test]
fn the_description_covers_the_questions_and_the_unknown_zone() {
    let description = CurrentTimeTool.description();

    for word in [
        "time",
        "date",
        "day of the week",
        "never guess",
        "Etc/Unknown",
    ] {
        assert!(description.contains(word), "{word}");
    }
}

#[test]
fn the_rule_accepts_empty_arguments_and_rejects_unknown_ones() {
    let rule = allowed_tool("current_time").expect("current_time is allowed");

    assert!(rule.accepts_arguments(&json!({})));
    assert!(!rule.accepts_arguments(&json!({"zone": "UTC"})));
    assert!(!rule.accepts_arguments(&json!(null)));
}
