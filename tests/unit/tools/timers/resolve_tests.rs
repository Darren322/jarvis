use jiff::Zoned;

use super::{MAX_TIMER_MINUTES, MIN_TIMER_MINUTES, ResolveError, TimeRequest, resolve, spoken_at};

fn zoned(text: &str) -> Zoned {
    text.parse().expect("the test instant is valid")
}

fn unix_ms(text: &str) -> i64 {
    zoned(text).timestamp().as_millisecond()
}

const HOUR_MS: i64 = 3_600_000;

#[test]
fn a_request_resolves_to_its_end_instant_and_spoken_time() {
    // (name, now, request, end instant, spoken)
    let cases = [
        (
            "25 minutes from now",
            "2026-11-22T18:00:00[Asia/Singapore]",
            TimeRequest::InMinutes(25),
            "2026-11-22T18:25:00[Asia/Singapore]",
            "6:25 pm today",
        ),
        (
            "11 pm at 10:59 pm is today",
            "2026-11-22T22:59:00[Asia/Singapore]",
            TimeRequest::AtClock("23:00"),
            "2026-11-22T23:00:00[Asia/Singapore]",
            "11:00 pm today",
        ),
        (
            "11 pm at 11:00 pm is tomorrow",
            "2026-11-22T23:00:00[Asia/Singapore]",
            TimeRequest::AtClock("23:00"),
            "2026-11-23T23:00:00[Asia/Singapore]",
            "11:00 pm tomorrow",
        ),
        (
            "11 pm at 11:01 pm is tomorrow",
            "2026-11-22T23:01:00[Asia/Singapore]",
            TimeRequest::AtClock("23:00"),
            "2026-11-23T23:00:00[Asia/Singapore]",
            "11:00 pm tomorrow",
        ),
        (
            "a clock time later in the same minute is tomorrow",
            "2026-11-22T23:00:20[Asia/Singapore]",
            TimeRequest::AtClock("23:00"),
            "2026-11-23T23:00:00[Asia/Singapore]",
            "11:00 pm tomorrow",
        ),
        (
            "minutes across midnight",
            "2026-11-22T23:50:00[Asia/Singapore]",
            TimeRequest::InMinutes(25),
            "2026-11-23T00:15:00[Asia/Singapore]",
            "12:15 am tomorrow",
        ),
        (
            "midnight at 11:59 pm is tomorrow",
            "2026-11-22T23:59:00[Asia/Singapore]",
            TimeRequest::AtClock("00:00"),
            "2026-11-23T00:00:00[Asia/Singapore]",
            "12:00 am tomorrow",
        ),
        (
            "a clock time across a month end",
            "2026-11-30T23:30:00[Asia/Singapore]",
            TimeRequest::AtClock("07:15"),
            "2026-12-01T07:15:00[Asia/Singapore]",
            "7:15 am tomorrow",
        ),
        (
            "minutes across a year end",
            "2026-12-31T23:00:00[Asia/Singapore]",
            TimeRequest::InMinutes(120),
            "2027-01-01T01:00:00[Asia/Singapore]",
            "1:00 am tomorrow",
        ),
        (
            "a half-hour zone, still ahead",
            "2026-11-22T10:15:00[Asia/Kolkata]",
            TimeRequest::AtClock("10:30"),
            "2026-11-22T10:30:00[Asia/Kolkata]",
            "10:30 am today",
        ),
        (
            "a half-hour zone, already passed",
            "2026-11-22T10:45:00[Asia/Kolkata]",
            TimeRequest::AtClock("10:30"),
            "2026-11-23T10:30:00[Asia/Kolkata]",
            "10:30 am tomorrow",
        ),
        (
            "a half-hour zone, minutes",
            "2026-11-22T10:45:00[Asia/Kolkata]",
            TimeRequest::InMinutes(45),
            "2026-11-22T11:30:00[Asia/Kolkata]",
            "11:30 am today",
        ),
        (
            "a clock time the spring-forward skips moves forward",
            "2026-03-08T01:00:00[America/New_York]",
            TimeRequest::AtClock("02:30"),
            "2026-03-08T03:30:00[America/New_York]",
            "3:30 am today",
        ),
        (
            "minutes over the spring-forward are real minutes",
            "2026-03-08T01:00:00[America/New_York]",
            TimeRequest::InMinutes(90),
            "2026-03-08T03:30:00[America/New_York]",
            "3:30 am today",
        ),
        (
            "a clock time on the fall-back day",
            "2026-11-01T00:30:00[America/New_York]",
            TimeRequest::AtClock("23:00"),
            "2026-11-01T23:00:00[America/New_York]",
            "11:00 pm today",
        ),
    ];

    for (name, now, request, end, spoken) in cases {
        let resolved = resolve(&zoned(now), request).expect(name);
        assert_eq!(resolved.ends_at_unix_ms, unix_ms(end), "{name}: end");
        assert_eq!(resolved.spoken, spoken, "{name}: spoken");
    }
}

#[test]
fn a_clock_time_is_that_time_on_the_clock_not_24_hours_later() {
    // The day after spring-forward has 23 hours; the day before fall-back, 25.
    let spring = resolve(
        &zoned("2026-03-07T23:00:00[America/New_York]"),
        TimeRequest::AtClock("23:00"),
    )
    .expect("resolves");
    assert_eq!(
        spring.ends_at_unix_ms - unix_ms("2026-03-07T23:00:00[America/New_York]"),
        23 * HOUR_MS
    );

    let fall = resolve(
        &zoned("2026-10-31T23:00:00[America/New_York]"),
        TimeRequest::AtClock("23:00"),
    )
    .expect("resolves");
    assert_eq!(
        fall.ends_at_unix_ms - unix_ms("2026-10-31T23:00:00[America/New_York]"),
        25 * HOUR_MS
    );
}

#[test]
fn minutes_are_accepted_from_1_to_1440_and_nothing_else_is() {
    assert_eq!(MIN_TIMER_MINUTES, 1);
    assert_eq!(MAX_TIMER_MINUTES, 1440);
    let now = zoned("2026-11-22T18:00:00[Asia/Singapore]");
    for minutes in [1, 2, 1439, 1440] {
        assert!(
            resolve(&now, TimeRequest::InMinutes(minutes)).is_ok(),
            "{minutes}"
        );
    }
    for minutes in [i64::MIN, -1, 0, 1441, i64::MAX] {
        assert!(
            matches!(
                resolve(&now, TimeRequest::InMinutes(minutes)),
                Err(ResolveError::MinutesOutOfRange { minutes: m }) if m == minutes
            ),
            "{minutes}"
        );
    }
}

#[test]
fn a_full_day_ends_at_the_same_clock_time_tomorrow() {
    let resolved = resolve(
        &zoned("2026-11-22T18:00:00[Asia/Singapore]"),
        TimeRequest::InMinutes(1440),
    )
    .expect("resolves");
    assert_eq!(resolved.spoken, "6:00 pm tomorrow");
}

#[test]
fn only_hh_mm_in_24_hour_time_is_a_clock_time() {
    let now = zoned("2026-11-22T18:00:00[Asia/Singapore]");
    for at in ["00:00", "07:30", "12:00", "23:00", "23:59", " 23:00 "] {
        assert!(resolve(&now, TimeRequest::AtClock(at)).is_ok(), "{at:?}");
    }
    for at in [
        "",
        "7:30",
        "23:0",
        "24:00",
        "23:60",
        "2300",
        "11 pm",
        "11:00 pm",
        "23:00:00",
        "ab:cd",
        "+1:00",
        "-1:00",
        "23.00",
        ":",
        "１１:００",
    ] {
        let error = resolve(&now, TimeRequest::AtClock(at)).expect_err(at);
        assert!(
            matches!(&error, ResolveError::NotAClockTime { given } if given == at),
            "{at:?}"
        );
        assert!(error.to_string().contains("HH:MM"), "{error}");
    }
}

#[test]
fn a_time_beyond_tomorrow_names_its_day() {
    let now = zoned("2026-11-22T18:00:00[Asia/Singapore]");
    let in_three_days = unix_ms("2026-11-25T09:05:00[Asia/Singapore]");
    assert_eq!(
        spoken_at(&now, in_three_days).expect("speaks"),
        "9:05 am on Wednesday 25 November"
    );
    assert_eq!(
        spoken_at(&now, unix_ms("2026-11-22T06:00:00[Asia/Singapore]")).expect("speaks"),
        "6:00 am today"
    );
}
