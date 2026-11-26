use orb_protocol::MAX_TIMER_LABEL_CHARS;
use rig_agent::tool::Tool;
use rig_core::serde_json::{Value, from_value, json};

use super::{SetTimerArgs, SetTimerTool, TimerWhen};
use crate::features::timers::{CancelOutcome, TimerServiceError, channel};
use crate::storage::{Timer, TimerState};
use crate::tools::allowed_tool;
use crate::tools::timers::TimerToolError;
use crate::tools::timers::resolve::ResolveError;
use crate::tools::timers::tests::{MINUTE_MS, Service, now, now_ms};

fn args(value: Value) -> SetTimerArgs {
    from_value(value).expect("the arguments are accepted")
}

#[test]
fn exactly_one_of_minutes_seconds_or_at_is_accepted_with_an_optional_label() {
    let accepted = [
        json!({"minutes": 10}),
        json!({"at": "23:00"}),
        json!({"label": "Time to do work", "minutes": 25}),
        json!({"label": "Time to sleep", "at": "23:00"}),
        json!({"label": null, "minutes": 5}),
        json!({"seconds": 5}),
        json!({"label": "Tea", "seconds": 90}),
    ];
    let rejected = [
        json!({}),
        json!({"label": "Time to sleep"}),
        json!({"minutes": 10, "at": "23:00"}),
        json!({"label": "x", "minutes": 10, "at": "23:00"}),
        json!({"minutes": null, "at": null}),
        json!({"minutes": "10"}),
        json!({"minutes": 2.5}),
        json!({"at": 2300}),
        json!({"minutes": 10, "extra": 1}),
        json!({"minutes": 1, "seconds": 60}),
        json!({"seconds": 30, "at": "23:00"}),
        json!({"minutes": 1, "seconds": 60, "at": "23:00"}),
        json!({"seconds": "5"}),
        json!({"seconds": 2.5}),
        json!(null),
        json!([]),
    ];
    let rule = allowed_tool("set_timer").expect("set_timer is allowed");
    for arguments in accepted {
        assert!(
            from_value::<SetTimerArgs>(arguments.clone()).is_ok(),
            "{arguments}"
        );
        assert!(rule.accepts_arguments(&arguments), "{arguments}");
    }
    for arguments in rejected {
        assert!(
            from_value::<SetTimerArgs>(arguments.clone()).is_err(),
            "{arguments}"
        );
        assert!(!rule.accepts_arguments(&arguments), "{arguments}");
    }
}

#[test]
fn the_arguments_say_which_of_minutes_seconds_or_at_was_given() {
    assert_eq!(args(json!({"minutes": 10})).when, TimerWhen::InMinutes(10));
    assert_eq!(args(json!({"seconds": 5})).when, TimerWhen::InSeconds(5));
    assert_eq!(
        args(json!({"at": "23:00"})).when,
        TimerWhen::AtClock("23:00".to_owned())
    );
    let both = from_value::<SetTimerArgs>(json!({"minutes": 1, "at": "23:00"}))
        .expect_err("both is refused");
    assert!(both.to_string().contains("exactly one"), "{both}");
    let neither = from_value::<SetTimerArgs>(json!({"label": "x"})).expect_err("neither");
    assert!(neither.to_string().contains("exactly one"), "{neither}");
}

#[test]
fn the_name_schema_and_description_are_for_the_model() {
    let tool = SetTimerTool::new(channel().0);
    assert_eq!(SetTimerTool::NAME, "set_timer");

    let schema = tool.parameters();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], json!(false));
    let properties = schema["properties"].as_object().expect("properties");
    assert_eq!(
        properties.keys().map(String::as_str).collect::<Vec<_>>(),
        ["at", "label", "minutes", "seconds"]
    );
    assert_eq!(properties["label"]["type"], "string");
    assert_eq!(properties["minutes"]["type"], "integer");
    assert_eq!(properties["seconds"]["type"], "integer");
    assert_eq!(properties["at"]["type"], "string");
    assert!(
        schema.get("required").is_none(),
        "every property is optional"
    );
    // Gemma's chat template cannot show these.
    let text = schema.to_string();
    for keyword in ["anyOf", "oneOf", "allOf", "$ref", "not"] {
        assert!(!text.contains(&format!("\"{keyword}\"")), "{keyword}");
    }

    let description = tool.description();
    for phrase in [
        "Use it for every request to set a timer, and never answer one without it",
        "set a timer for 10 minutes",
        "10-minute timer on the screen",
        "in 25 minutes, time to do work",
        "set a timer for 30 seconds",
        "set a timer for 11 pm, time to sleep",
        "\"23:00\"",
        "1 to 1440",
    ] {
        assert!(description.contains(phrase), "{phrase}: {description}");
    }
    // What to say after the call is in the result's note: Gemma said such lines
    // without calling the tool.
    for instruction in ["never say", "say the", "say so", "waiting for"] {
        assert!(
            !description.contains(instruction),
            "{instruction}: {description}"
        );
    }
}

#[tokio::test]
async fn a_duration_is_only_proposed_and_the_timer_waits_for_a_yes() {
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());

    let output = tool
        .propose(
            &now(),
            args(json!({"label": "Time to do work", "minutes": 25})),
        )
        .await
        .expect("proposed");

    assert_eq!(output["status"], "proposed_not_set_yet");
    assert_eq!(output["label"], "Time to do work");
    assert_eq!(output["ends_at"], "6:25 pm today");
    let note = output["note"].as_str().expect("a note");
    assert!(note.contains("NOT set yet"), "{note}");
    assert!(note.contains("30 seconds"), "{note}");
    assert!(note.contains("Never say it is set"), "{note}");

    let shown = service.shown_until(|timers| timers.len() == 1).await;
    assert_eq!(shown[0].state, TimerState::Pending);
    assert_eq!(shown[0].label.as_deref(), Some("Time to do work"));
    assert_eq!(shown[0].ends_at_unix_ms, now_ms() + 25 * MINUTE_MS);
}

#[tokio::test]
async fn a_clock_time_is_proposed_for_today_or_tomorrow() {
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());

    let today = tool
        .propose(
            &now(),
            args(json!({"label": "Time to sleep", "at": "23:00"})),
        )
        .await
        .expect("proposed");
    // 6 pm has passed at 18:00 exactly: a clock time equal to now is tomorrow.
    let tomorrow = tool
        .propose(&now(), args(json!({"at": "18:00"})))
        .await
        .expect("proposed");

    assert_eq!(today["ends_at"], "11:00 pm today");
    assert_eq!(tomorrow["ends_at"], "6:00 pm tomorrow");
    assert_eq!(tomorrow["label"], Value::Null);
    // A new proposal replaces the one still pending (D23): only the 6 pm one waits.
    let shown = service
        .shown_until(|timers| timers.len() == 1 && timers[0].label.is_none())
        .await;
    assert_eq!(shown[0].state, TimerState::Pending);
}

#[tokio::test]
async fn a_timer_is_never_running_until_it_is_confirmed() {
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());
    tool.propose(&now(), args(json!({"minutes": 10})))
        .await
        .expect("proposed");

    let shown = service.shown_until(|timers| !timers.is_empty()).await;

    assert!(shown.iter().all(|timer| timer.state != TimerState::Running));
}

#[tokio::test]
async fn five_timers_are_proposed_and_the_sixth_is_refused_and_creates_nothing() {
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());
    // At most one timer is pending (D23), so the limit is filled with running ones.
    for minutes in 1..=5 {
        service.run_timer(None, minutes).await;
    }

    let error = tool
        .propose(&now(), args(json!({"minutes": 6})))
        .await
        .expect_err("the sixth is refused");

    assert!(
        matches!(error, TimerToolError::TooManyTimers { limit: 5 }),
        "{error:?}"
    );
    assert!(error.to_string().contains("nothing was created"));
    let CancelOutcome::NoMatch(left) = service
        .handle
        .cancel_matching(Some("no-such-label".to_owned()))
        .await
        .expect("answered")
    else {
        panic!("no label matches");
    };
    assert_eq!(left.len(), 5);
}

#[tokio::test]
async fn a_label_of_60_characters_is_proposed_and_one_of_61_is_refused_first() {
    assert_eq!(MAX_TIMER_LABEL_CHARS, 60);
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());
    // Two bytes a character: the limit counts characters, not bytes.
    let at_limit = "é".repeat(60);
    let over_limit = "é".repeat(61);

    let kept = tool
        .propose(&now(), args(json!({"label": at_limit, "minutes": 5})))
        .await
        .expect("60 characters are accepted");
    let error = tool
        .propose(&now(), args(json!({"label": over_limit, "minutes": 5})))
        .await
        .expect_err("61 characters are refused");

    assert_eq!(kept["label"], json!(at_limit));
    assert!(
        matches!(error, TimerToolError::LabelTooLong { chars: 61 }),
        "{error:?}"
    );
    assert!(error.to_string().contains("shorter label"), "{error}");
    // Refused before `propose`: only the first timer exists.
    let CancelOutcome::NoMatch(stored) = service
        .handle
        .cancel_matching(Some("no-such-label".to_owned()))
        .await
        .expect("answered")
    else {
        panic!("no label matches");
    };
    assert_eq!(stored.len(), 1);
}

#[tokio::test]
async fn a_blank_label_is_no_label_and_a_label_is_trimmed() {
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());

    let blank = tool
        .propose(&now(), args(json!({"label": "   ", "minutes": 5})))
        .await
        .expect("proposed");
    let padded = tool
        .propose(&now(), args(json!({"label": "  Tea  ", "minutes": 5})))
        .await
        .expect("proposed");

    assert_eq!(blank["label"], Value::Null);
    assert_eq!(padded["label"], "Tea");
}

#[tokio::test]
async fn minutes_of_1_and_1440_are_proposed_and_0_and_1441_are_refused_without_a_timer() {
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());
    for minutes in [1, 1440] {
        tool.propose(&now(), args(json!({"minutes": minutes})))
            .await
            .unwrap_or_else(|error| panic!("{minutes}: {error}"));
    }

    for minutes in [0, 1441, -5] {
        let error = tool
            .propose(&now(), args(json!({"minutes": minutes})))
            .await
            .expect_err("out of range");
        assert!(
            matches!(
                error,
                TimerToolError::Resolve(ResolveError::MinutesOutOfRange { .. })
            ),
            "{minutes}: {error:?}"
        );
        assert!(error.to_string().contains("1 to 1440"), "{error}");
    }
    // Each proposal replaced the last (D23): the 1440-minute one is the one pending,
    // and none of the refused ones created a timer.
    let lasts_a_day =
        |timer: &Timer| timer.ends_at_unix_ms - timer.created_at_unix_ms == 1440 * 60_000;
    let shown = service
        .shown_until(|timers| timers.iter().any(lasts_a_day))
        .await;
    assert_eq!(shown.len(), 1);
}

#[tokio::test]
async fn sixty_seconds_is_proposed_as_a_minute_and_five_seconds_is_refused_without_a_timer() {
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());

    let error = tool
        .propose(&now(), args(json!({"seconds": 5})))
        .await
        .expect_err("under a minute");
    assert!(
        matches!(
            error,
            TimerToolError::Resolve(ResolveError::NotWholeMinutes { seconds: 5 })
        ),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("the shortest timer is 1 minute"),
        "{error}"
    );
    assert!(service.handle.shown().is_empty(), "nothing was created");
    // The model is told the message itself, not Rig's redacted default.
    let message = error.to_string();
    assert_eq!(
        tool.map_error(error).model_feedback(),
        Some(message.as_str())
    );

    tool.propose(&now(), args(json!({"seconds": 60})))
        .await
        .expect("a whole minute is proposed");
    let lasts_a_minute =
        |timer: &Timer| timer.ends_at_unix_ms - timer.created_at_unix_ms == MINUTE_MS;
    let shown = service
        .shown_until(|timers| timers.iter().any(lasts_a_minute))
        .await;
    assert_eq!(shown.len(), 1);
}

#[tokio::test]
async fn a_clock_time_that_is_not_hh_mm_is_refused_with_the_form_to_use() {
    let service = Service::start().await;
    let tool = SetTimerTool::new(service.handle.clone());

    let error = tool
        .propose(&now(), args(json!({"at": "11 pm"})))
        .await
        .expect_err("not HH:MM");

    assert!(
        matches!(
            error,
            TimerToolError::Resolve(ResolveError::NotAClockTime { .. })
        ),
        "{error:?}"
    );
    assert!(error.to_string().contains("HH:MM"), "{error}");
    assert!(error.to_string().contains("\"23:00\""), "{error}");
}

#[tokio::test]
async fn without_a_running_service_the_tool_says_the_timers_are_unavailable() {
    let (handle, inbox, _events) = channel();
    drop(inbox);
    let tool = SetTimerTool::new(handle);

    let error = tool
        .propose(&now(), args(json!({"minutes": 5})))
        .await
        .expect_err("nothing is serving");

    assert!(
        matches!(
            error,
            TimerToolError::Service(TimerServiceError::Unavailable)
        ),
        "{error:?}"
    );
}
